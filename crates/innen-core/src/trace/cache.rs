use super::lexical::lex;
use super::types::{Error, LexDoc, Locator, CACHE_LIMIT, CACHE_SCHEMA, GRAPH_LIMIT};
use crate::conversation::Source;
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Link {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) kind: String,
    pub(super) provenance: Option<String>,
    pub(super) valid_from: String,
    pub(super) valid_until: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct GraphCache {
    pub(super) schema: u32,
    pub(super) journal_sha256: String,
    pub(super) nodes: BTreeMap<String, Value>,
    pub(super) links: Vec<Link>,
    pub(super) docs: Vec<LexDoc>,
}

pub(super) fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error(format!(
            "{} exceeds {limit} byte bound",
            path.display()
        )));
    }
    Ok(bytes)
}
pub(super) fn private_dir(path: &Path) -> Result<(), Error> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
pub(super) fn atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error("cache path has no parent".into()))?;
    private_dir(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error(e.to_string()))?
        .as_nanos();
    let tmp = parent.join(format!(".tmp-{}-{nonce}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}
pub(super) fn cache_root(root: &Path) -> PathBuf {
    root.join(".innen/trace")
}

pub(super) fn graph(root: &Path, refresh: bool) -> Result<(GraphCache, bool, usize), Error> {
    // Never call Journal::open on this read path: it may quarantine input.
    let bytes = bounded(&root.join(".innen/journal.jsonl"), GRAPH_LIMIT)?;
    let hash = sha256_hex(&bytes);
    let cached = cache_root(root).join(format!("graph-{hash}.json"));
    if !refresh {
        if let Ok(bytes) = bounded(&cached, CACHE_LIMIT) {
            if let Ok(found) = serde_json::from_slice::<GraphCache>(&bytes) {
                if found.schema == CACHE_SCHEMA && found.journal_sha256 == hash {
                    return Ok((found, true, bytes.len()));
                }
            }
        }
    }
    let text = std::str::from_utf8(&bytes).map_err(|e| Error(e.to_string()))?;
    let mut events = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: crate::journal::JournalEntry = serde_json::from_str(line)
            .map_err(|e| Error(format!("journal line {}: {e}", i + 1)))?;
        events
            .push(json!({"op":event.op,"payload":event.payload,"observed_utc":event.observed_utc}));
    }
    // Cache all validity intervals; evaluate time at each traversal, not only
    // when this journal-hash cache was first created.
    let materialized = crate::graph::materialize(&events, None, true);
    let nodes: BTreeMap<_, _> = materialized.nodes.into_iter().collect();
    let mut docs = Vec::new();
    for (id, node) in &nodes {
        // wiki-sync historically emitted `missing`; newer projections may use
        // `wiki_missing`. Either marker means the page is stale and must not
        // become a retrieval seed.
        if node.get("wiki_missing").and_then(Value::as_bool) == Some(true)
            || node.get("missing").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        // A source node created by wiki-sync is only a route target. Once its
        // owning page is deleted/retracted it must not remain a direct lexical
        // seed from stale journal materialization.
        if node.get("managed_by").and_then(Value::as_str) == Some(crate::wiki_graph::MANAGED_BY)
            && node.get("type").and_then(Value::as_str) == Some("Source")
            && !materialized.edges.iter().any(|edge| {
                !edge.retracted && edge.to == *id && edge.edge.to_string() == "DERIVED_FROM"
            })
        {
            continue;
        }
        let mut text = crate::task_entry::node_searchable_text(node);
        text.push(' ');
        text.push_str(id);
        docs.push(lex(id.clone(), &text));
    }
    let links = materialized
        .edges
        .into_iter()
        .filter(|e| !e.retracted)
        .map(|e| Link {
            from: e.from,
            to: e.to,
            kind: e.edge.to_string(),
            provenance: e.provenance,
            valid_from: e.valid_from,
            valid_until: e.valid_until,
        })
        .collect();
    let out = GraphCache {
        schema: CACHE_SCHEMA,
        journal_sha256: hash,
        nodes,
        links,
        docs,
    };
    atomic(&cached, &serde_json::to_vec(&out)?)?;
    Ok((out, false, bytes.len()))
}

pub(super) fn kind(node: &Value) -> String {
    node["type"].as_str().unwrap_or("").to_lowercase()
}
pub(super) fn locator(node: &Value, root: &Path) -> Option<Locator> {
    if let Some(value) = node.get("locator") {
        if let Ok(loc) = serde_json::from_value(value.clone()) {
            return Some(loc);
        }
    }
    if kind(node) == "conversation" {
        let id = node["id"].as_str()?;
        let mut parts = id.splitn(3, ':');
        if parts.next() == Some("conversation") {
            let source = parts.next()?;
            if Source::parse(source).is_ok() {
                return Some(Locator::Conversation {
                    source: source.into(),
                    session_id: parts.next()?.into(),
                    source_root: None,
                });
            }
        }
    }
    if kind(node) == "wiki" {
        return None;
    }
    let path = node
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| node.pointer("/provenance/path").and_then(Value::as_str))
        .or_else(|| node.get("provenance").and_then(Value::as_str))?;
    if path.starts_with("http://") || path.starts_with("https://") {
        return Some(Locator::Url { url: path.into() });
    }
    if path.starts_with('/') {
        return Some(Locator::File { path: path.into() });
    }
    if [
        "00-inbox/",
        "01-raw/",
        "02-wiki/",
        "03-output/",
        "04-index/",
    ]
    .iter()
    .any(|s| path.starts_with(s))
    {
        return Some(Locator::File {
            path: root.join(path).display().to_string(),
        });
    }
    None
}

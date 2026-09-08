use super::cache::{bounded, cache_root};
use super::source::{valid_key, SourceCache};
use super::types::{Error, Locator, CACHE_LIMIT, GRAPH_LIMIT};
use crate::conversation;
use crate::ids::sha256_hex;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Expand a selected record against current source bytes, never trust cached text alone.
pub fn expand(root: &Path, key: &str, record: usize, expected: &str) -> Result<Value, Error> {
    if !valid_key(key) || !valid_key(expected) {
        return Err(Error("invalid cache or record SHA-256".into()));
    }
    let bytes = bounded(
        &cache_root(root).join("sources").join(format!("{key}.json")),
        CACHE_LIMIT,
    )?;
    if sha256_hex(&bytes) != key {
        return Err(Error("source cache hash mismatch".into()));
    }
    let cached: SourceCache = serde_json::from_slice(&bytes)?;
    let passage = cached
        .passages
        .get(record)
        .ok_or_else(|| Error("unknown source record".into()))?;
    if passage.hash != expected {
        return Err(Error("expected record hash differs from receipt".into()));
    }
    let output = match &cached.locator {
        Locator::File { path } => {
            let start = passage
                .start_byte
                .ok_or_else(|| Error("missing source offset".into()))?;
            let end = passage
                .end_byte
                .ok_or_else(|| Error("missing source end".into()))?;
            if end < start || end - start > GRAPH_LIMIT {
                return Err(Error("invalid source range".into()));
            }
            let mut f = File::open(path)?;
            f.seek(SeekFrom::Start(start))?;
            let mut bytes = Vec::new();
            f.take(end - start).read_to_end(&mut bytes)?;
            if sha256_hex(&bytes) != expected {
                return Err(Error("source changed: record SHA-256 mismatch".into()));
            }
            json!({"role":passage.role,"content":String::from_utf8(bytes).map_err(|e|Error(e.to_string()))?})
        }
        Locator::Conversation {
            source,
            session_id,
            source_root,
        } => {
            let page = conversation::read(
                source_root.as_deref().map(Path::new),
                source,
                session_id,
                "dialogue",
                passage.line - 1,
                1,
            )
            .map_err(|e| Error(e.to_string()))?;
            let current = page
                .records
                .first()
                .ok_or_else(|| Error("source record missing".into()))?;
            if current.line != passage.line
                || sha256_hex(&serde_json::to_vec(&current.event)?) != expected
            {
                return Err(Error("source changed: record SHA-256 mismatch".into()));
            }
            current.event.clone()
        }
        Locator::Url { .. } => return Err(Error("remote expansion is not supported".into())),
    };
    let text = output["content"].as_str().unwrap_or("");
    if !crate::tap::scan_credentials(text).is_empty() {
        return Err(Error("credential-like source record withheld".into()));
    }
    Ok(
        json!({"resolution":"expanded","source":cached.locator,"source_path":cached.source_path,"line":passage.line,"end_line":passage.end_line,"record_sha256":expected,"record":output}),
    )
}

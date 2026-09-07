//! P1 query contract (Task 7, spec §3–§4).
//!
//! Pinned algorithm (implement exactly; fixtures are hand-run against this):
//!
//! 1. Replay: [`crate::graph::materialize`] journal events with `as_of`
//!    (`None` defaults to now here, then passes into `materialize()` as
//!    `Some`) and `include_expired` straight through.
//! 2. Index: build a throwaway in-RAM tantivy index from materialized nodes
//!    with [`crate::index::schema`] + [`crate::index::ensure_tokenizer`]
//!    (called once at index build before indexing; the same `Index`
//!    serves searching). Indexed text per node is `label`, or `label + " " + body` when a body
//!    exists. Nodes are added in sorted id order for determinism.
//! 3. Seeds: FTS top-20 over `body` (BM25) UNION exact node-id substring
//!    match (`node_id.contains(q)`; empty `q` matches nothing on either
//!    side — FTS is skipped and `contains("")` is not allowed to seed all).
//! 4. BFS: forward along `from -> to` over materialized edges, depth ≤ 3,
//!    skipping retracted rows, dangling endpoints, and edges rejected by
//!    [`crate::graph::validate`] (provenance read from the from-node's
//!    `provenance` field when present; core triples need none).
//! 5. Ranking: graph hits score `1.0 / (1.0 + depth)`; FTS hits score
//!    `bm25 / max_bm25_in_this_query` clamped to `0..=1` (`max == 0` or no
//!    hits → all `0.0`, never divide by zero). Graph hits sort by
//!    `(depth, node_id)`, FTS hits by `(score desc, node_id)`; graph block
//!    always before FTS block. `why` has a single winner: a node in both
//!    sets keeps the graph copy only. Note depth-0 seeds are graph hits,
//!    so an FTS hit that is also a seed is reported with `why: "graph"`.
//! 6. Shape: `kind` is the [`crate::graph::NodeType`] display name verbatim
//!    (unknown types stay `Custom`, reported as their stored string);
//!    `excerpt` is `label`, or `label + " " + first 200 chars of body`.
//! 7. Limit (`u16`, default 20, max 100): requested > 100 clamps to 100
//!    with a `truncated` warning; independently, cutting ranked hits down
//!    to the effective limit also raises `truncated` (once). `limit: 0`
//!    is pinned to empty `hits` with `truncated` (always, even when the
//!    query would otherwise return nothing).
//! 8. Warnings (in this push order): `truncated`, `quarantined_skipped`
//!    (a non-empty `<root>/.innen/quarantine/*.jsonl` exists at query
//!    open — FTS itself always scans all indexed nodes), `time_blind_fts`
//!    (≥1 FTS-only hit survived dedup pre-limit; FTS ignores `as_of`).
//! 9. Embedding hook: absent in P1 — no config surface, no call. The
//!    degrade path IS the current behavior (lexical + graph only).
//!
//! Reuse boundary: replay via `materialize`, adjacency via `validate`,
//! node kinds via `NodeType`, journal via `Journal`, time via
//! `observed_utc_now`. Event identity/canonicalization stays in
//! [`crate::ids`] on the journal write path; query never re-derives ids.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value as _;
use tantivy::{doc, Index, TantivyDocument};

use crate::graph::{materialize, validate, Endpoint, NodeType};
use crate::index::{ensure_tokenizer, schema};
use crate::journal::{observed_utc_now, Journal};

/// Maximum accepted limit; requests above this clamp with `truncated`.
pub const MAX_LIMIT: u16 = 100;
/// Default limit when the caller passes none.
pub const DEFAULT_LIMIT: u16 = 20;
/// FTS seed/hit cap per query.
pub const FTS_TOP_K: usize = 20;
/// BFS radius from seeds, inclusive.
pub const MAX_DEPTH: usize = 3;
/// Excerpt body budget, in chars (not bytes).
pub const EXCERPT_BODY_CHARS: usize = 200;

/// Input to [`query`]. `--as-of` maps to `as_of` (defaults to now),
/// `--include-expired` maps to `include_expired`.
#[derive(Debug, Clone)]
pub struct QueryParams {
    pub q: String,
    pub as_of: Option<String>,
    pub limit: u16,
    pub include_expired: bool,
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            q: String::new(),
            as_of: None,
            limit: DEFAULT_LIMIT,
            include_expired: false,
        }
    }
}

/// One ranked hit. Field order is the wire order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub node_id: String,
    pub kind: String,
    pub label: String,
    pub excerpt: String,
    pub score: f64,
    pub why: String,
}

/// Ranked output for one query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryOutput {
    pub hits: Vec<Hit>,
    pub warnings: Vec<String>,
}

/// Query failure: journal I/O or tantivy errors. A lone unparseable query
/// string is NOT an error — it degrades to zero FTS hits.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("journal: {0}")]
    Journal(#[from] crate::journal::JournalError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tantivy: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

fn node_text<'a>(node: &'a serde_json::Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// [`NodeType`] display name verbatim; missing/non-string `type` → `"Custom"`.
fn node_kind(node: &serde_json::Value) -> String {
    match node.get("type").and_then(|v| v.as_str()) {
        // `FromStr` is infallible by construction (unknown → `Custom`).
        Some(s) => s.parse::<NodeType>().unwrap().to_string(),
        None => "Custom".to_string(),
    }
}

/// `label`, or `label + " " + first 200 chars of body`.
// Widened to `pub(crate)` so `parity::search` reuses the same helper (no dup).
pub(crate) fn excerpt_of(label: &str, body: &str) -> String {
    let short: String = body.chars().take(EXCERPT_BODY_CHARS).collect();
    match (label.is_empty(), short.is_empty()) {
        (true, _) => short,
        (false, true) => label.to_string(),
        (false, false) => format!("{label} {short}"),
    }
}

/// Non-empty quarantine spill files left by [`Journal::open`].
fn has_quarantine(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root.join(".innen").join("quarantine")) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        p.extension().is_some_and(|x| x == "jsonl")
            && p.is_file()
            && std::fs::metadata(&p).is_ok_and(|m| m.len() > 0)
    })
}

/// Run the pinned P1 query against the journal at `root`.
///
/// `limit: 0` is pinned to empty `hits` with a `truncated` warning (a
/// degenerate request for zero rows always reports truncation, even when
/// the query would otherwise return nothing).
pub fn query(root: impl AsRef<Path>, params: &QueryParams) -> Result<QueryOutput, QueryError> {
    let root = root.as_ref();
    let mut truncated = false;
    let effective_limit: usize = if params.limit > MAX_LIMIT {
        truncated = true;
        MAX_LIMIT as usize
    } else {
        params.limit as usize
    };
    // Pin: `limit: 0` always truncates (empty hits + warning).
    if params.limit == 0 {
        truncated = true;
    }

    // Journal open also performs the quarantine scan; real I/O errors are fatal.
    let journal = Journal::open(root)?;
    let mut warnings: Vec<String> = Vec::new();
    if has_quarantine(root) {
        warnings.push("quarantined_skipped".to_string());
    }

    let entries = journal.read_all()?;
    let events: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "op": e.op,
                "payload": e.payload,
                "observed_utc": e.observed_utc,
            })
        })
        .collect();
    // `--as-of` defaults to now and passes into `materialize()`.
    let as_of = params.as_of.clone().unwrap_or_else(observed_utc_now);
    let materialized = materialize(&events, Some(&as_of), params.include_expired);

    // --- FTS over a throwaway in-RAM index. ---
    let index_schema = schema();
    let body_field = index_schema.get_field("body").expect("schema defines body");
    let id_field = index_schema.get_field("id").expect("schema defines id");
    let index = Index::create_in_ram(index_schema);
    // Single tokenizer registration: the same `Index` serves both indexing
    // (below) and searching (reader/searcher); its TokenizerManager is
    // shared so one idempotent register before first use covers both.
    ensure_tokenizer(&index);
    {
        let mut writer = index.writer(15_000_000)?;
        let mut ids: Vec<&String> = materialized.nodes.keys().collect();
        ids.sort();
        for id in ids {
            let node = &materialized.nodes[id];
            let text = crate::task_entry::node_searchable_text(node);
            writer.add_document(doc!(
                body_field => text,
                id_field => id.clone(),
            ))?;
        }
        writer.commit()?;
    }
    let reader = index.reader()?;
    let searcher = reader.searcher();

    // FTS top-20; unparseable query degrades to zero hits, never an error.
    let mut fts_hits: Vec<(String, f64)> = Vec::new();
    if !params.q.is_empty() {
        let parser = QueryParser::for_index(&index, vec![body_field]);
        let query_res = parser.parse_query(&params.q).or_else(|_| {
            let sanitized: String = params
                .q
                .chars()
                .map(|c| {
                    if matches!(
                        c,
                        '-' | '+'
                            | '!'
                            | '('
                            | ')'
                            | ':'
                            | '^'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '~'
                            | '*'
                            | '?'
                    ) {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
            parser.parse_query(&sanitized)
        });
        if let Ok(parsed) = query_res {
            if let Ok(top) =
                searcher.search(&parsed, &TopDocs::with_limit(FTS_TOP_K).order_by_score())
            {
                let mut scored: Vec<(String, f32)> = Vec::new();
                for (bm25, addr) in top {
                    if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
                        if let Some(id) = doc.get_first(id_field).and_then(|v| v.as_str()) {
                            scored.push((id.to_string(), bm25));
                        }
                    }
                }
                let max = scored.iter().map(|(_, s)| *s).fold(0.0_f32, f32::max);
                for (id, bm25) in scored {
                    let norm = if max == 0.0 {
                        0.0
                    } else {
                        (f64::from(bm25) / f64::from(max)).clamp(0.0, 1.0)
                    };
                    fts_hits.push((id, norm));
                }
                fts_hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            }
        }
    }

    // --- Seeds: FTS ids + exact node-id/label substring match. ---
    let mut seeds: HashSet<String> = HashSet::new();
    for (id, _) in &fts_hits {
        seeds.insert(id.clone());
    }
    if !params.q.is_empty() {
        for (id, node) in &materialized.nodes {
            let label = node_text(node, "label");
            if id.contains(params.q.as_str()) || label.contains(params.q.as_str()) {
                seeds.insert(id.clone());
            }
        }
    }

    // --- Forward BFS (from -> to) over as_of-filtered, non-retracted edges. ---
    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &materialized.edges {
        if edge.retracted {
            continue;
        }
        let (Some(from_node), Some(to_node)) = (
            materialized.nodes.get(&edge.from),
            materialized.nodes.get(&edge.to),
        ) else {
            continue; // dangling: no Hit could be formed for the far end
        };
        let from_ty: NodeType = node_text(from_node, "type").parse().unwrap();
        let to_ty: NodeType = node_text(to_node, "type").parse().unwrap();
        // Edge assertions carry their own provenance. Fall back to the source
        // node only for legacy rows that predate edge provenance.
        let provenance = edge
            .provenance
            .as_deref()
            .or_else(|| from_node.get("provenance").and_then(|v| v.as_str()));
        if validate(&from_ty, &edge.edge, &Endpoint::Node(to_ty), provenance).is_err() {
            continue;
        }
        successors
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    for list in successors.values_mut() {
        list.sort_unstable();
        list.dedup();
    }
    let mut ordered_seeds: Vec<&String> = seeds.iter().collect();
    ordered_seeds.sort();
    let mut depth: HashMap<&str, usize> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for seed in ordered_seeds {
        if !depth.contains_key(seed.as_str()) {
            depth.insert(seed.as_str(), 0);
            queue.push_back(seed.as_str());
        }
    }
    while let Some(id) = queue.pop_front() {
        let d = depth[&id];
        if d >= MAX_DEPTH {
            continue;
        }
        if let Some(nexts) = successors.get(id) {
            for nxt in nexts {
                if !depth.contains_key(nxt) {
                    depth.insert(nxt, d + 1);
                    queue.push_back(nxt);
                }
            }
        }
    }

    // --- Ranking: graph block, then surviving FTS block. ---
    let mut ranked: Vec<(&str, f64, &str)> = Vec::new(); // (node_id, score, why)
    let mut graph_order: Vec<(&str, usize)> = depth.iter().map(|(id, d)| (*id, *d)).collect();
    graph_order.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    for (id, d) in graph_order {
        ranked.push((id, 1.0 / (1.0 + d as f64), "graph"));
    }
    let graph_ids: HashSet<&str> = depth.keys().copied().collect();
    let mut fts_survivors = 0;
    for (id, norm) in &fts_hits {
        if graph_ids.contains(id.as_str()) {
            continue; // single winner: graph copy only
        }
        ranked.push((id.as_str(), *norm, "fts"));
        fts_survivors += 1;
    }
    if fts_survivors > 0 {
        warnings.push("time_blind_fts".to_string());
    }
    if ranked.len() > effective_limit {
        truncated = true;
    }
    ranked.truncate(effective_limit);
    if truncated {
        warnings.insert(0, "truncated".to_string());
    }

    let mut hits = Vec::with_capacity(ranked.len());
    for (id, score, why) in ranked {
        let node = &materialized.nodes[id];
        let label = node_text(node, "label").to_string();
        hits.push(Hit {
            node_id: id.to_string(),
            kind: node_kind(node),
            excerpt: excerpt_of(&label, node_text(node, "body")),
            label,
            score,
            why: why.to_string(),
        });
    }
    Ok(QueryOutput { hits, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_combines_label_and_200_char_body() {
        assert_eq!(excerpt_of("L", ""), "L");
        assert_eq!(excerpt_of("", "B"), "B");
        assert_eq!(excerpt_of("L", "B"), "L B");
        let long = "字".repeat(250);
        let got = excerpt_of("L", &long);
        assert_eq!(got, format!("L {}", "字".repeat(200)));
        assert_eq!(got.chars().count(), 202);
    }

    #[test]
    fn kind_verbatim_and_custom_passthrough() {
        let task = serde_json::json!({"type": "Task"});
        assert_eq!(node_kind(&task), "Task");
        let lower = serde_json::json!({"type": "task"});
        assert_eq!(node_kind(&lower), "Task");
        let custom = serde_json::json!({"type": "Wiki"});
        assert_eq!(node_kind(&custom), "Wiki");
        let missing = serde_json::json!({});
        assert_eq!(node_kind(&missing), "Custom");
    }

    #[test]
    fn quarantine_detected_only_with_nonempty_spill() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!has_quarantine(dir.path()));
        let qdir = dir.path().join(".innen").join("quarantine");
        std::fs::create_dir_all(&qdir).expect("mkdir");
        assert!(!has_quarantine(dir.path()));
        std::fs::write(qdir.join("2026-09-04.jsonl"), "").expect("write");
        assert!(!has_quarantine(dir.path()));
        std::fs::write(qdir.join("2026-09-04.jsonl"), "bad line\n").expect("write");
        assert!(has_quarantine(dir.path()));
    }

    #[test]
    fn limit_one_cuts_ranked_hits_with_truncated_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        for (id, label) in [
            ("a:1", "hello one"),
            ("a:2", "hello two"),
            ("a:3", "hello three"),
        ] {
            journal
                .append(
                    "node.upsert",
                    &serde_json::json!({"id": id, "type": "Task", "label": label}),
                )
                .expect("append");
        }
        let params = QueryParams {
            q: "hello".to_string(),
            as_of: Some("2026-09-03T00:00:00Z".to_string()),
            limit: 1,
            include_expired: false,
        };
        let out = query(dir.path(), &params).expect("query");
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].node_id, "a:1");
        assert_eq!(out.warnings, vec!["truncated".to_string()]);
    }

    #[test]
    fn limit_zero_pinned_to_empty_with_truncated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        journal
            .append(
                "node.upsert",
                &serde_json::json!({"id": "a:1", "type": "Task", "label": "hello one"}),
            )
            .expect("append");
        // Non-empty query: empty hits + truncated.
        let params = QueryParams {
            q: "hello".to_string(),
            as_of: Some("2026-09-03T00:00:00Z".to_string()),
            limit: 0,
            include_expired: false,
        };
        let out = query(dir.path(), &params).expect("query");
        assert!(out.hits.is_empty());
        assert_eq!(out.warnings, vec!["truncated".to_string()]);
        // Empty query: still empty hits + truncated (pinned, not conditional).
        let empty_params = QueryParams {
            q: "qqqzzzqqq".to_string(),
            as_of: Some("2026-09-03T00:00:00Z".to_string()),
            limit: 0,
            include_expired: false,
        };
        let empty_out = query(dir.path(), &empty_params).expect("query");
        assert!(empty_out.hits.is_empty());
        assert_eq!(empty_out.warnings, vec!["truncated".to_string()]);
    }
}

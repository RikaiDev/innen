//! Parity guide/search/status/timeline (Task 9).
//!
//! `search` is lexical-only over a throwaway in-RAM tantivy index (no graph
//! walk); `status` materializes the journal plus a line count; `timeline`
//! reads journal lines raw in order.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value as _;
use tantivy::{doc, Index, TantivyDocument};

use crate::graph::materialize;
use crate::index::{ensure_tokenizer, schema};
use crate::journal::Journal;
use crate::query::excerpt_of;

/// Short navigation text: what innen is, start with query, core commands.
pub fn guide_text() -> &'static str {
    "innen is a local-first knowledge graph over an append-only journal.\n\
     Start with query to ask the graph (lexical + graph walk).\n\
     Core commands: query, search, status, timeline, guide.\n\
     Use search for lexical-only hits, status for counts, timeline for history."
}

pub struct SearchHit {
    pub node_id: String,
    pub excerpt: String,
}

fn node_text<'a>(node: &'a serde_json::Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Lexical-only search over tantivy `body` (label [+ body]); no graph walk.
/// Cost: per-call throwaway in-RAM index rebuild O(n); persistent-index reads are a later phase.
pub fn search(root: &Path, keyword: &str, limit: u16) -> Vec<SearchHit> {
    if keyword.is_empty() || limit == 0 {
        return Vec::new();
    }
    let Ok(journal) = Journal::open(root) else {
        return Vec::new();
    };
    let Ok(entries) = journal.read_all() else {
        return Vec::new();
    };
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
    let materialized = materialize(&events, None, true);

    let index_schema = schema();
    let Ok(body_field) = index_schema.get_field("body") else {
        return Vec::new();
    };
    let Ok(id_field) = index_schema.get_field("id") else {
        return Vec::new();
    };
    let index = Index::create_in_ram(index_schema);
    ensure_tokenizer(&index);
    {
        let Ok(mut writer) = index.writer(15_000_000) else {
            return Vec::new();
        };
        let mut ids: Vec<&String> = materialized.nodes.keys().collect();
        ids.sort();
        for id in ids {
            let node = &materialized.nodes[id];
            let label = node_text(node, "label");
            let body = node_text(node, "body");
            let text = if body.is_empty() {
                label.to_string()
            } else if label.is_empty() {
                body.to_string()
            } else {
                format!("{label} {body}")
            };
            if writer
                .add_document(doc!(
                    body_field => text,
                    id_field => id.clone(),
                ))
                .is_err()
            {
                return Vec::new();
            }
        }
        if writer.commit().is_err() {
            return Vec::new();
        }
    }
    let Ok(reader) = index.reader() else {
        return Vec::new();
    };
    let searcher = reader.searcher();
    let parser = QueryParser::for_index(&index, vec![body_field]);
    let Ok(parsed) = parser.parse_query(keyword) else {
        return Vec::new();
    };
    let Ok(top) = searcher.search(
        &parsed,
        &TopDocs::with_limit(limit as usize).order_by_score(),
    ) else {
        return Vec::new();
    };
    let mut scored: Vec<(String, f32)> = Vec::new();
    for (score, addr) in top {
        if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
            if let Some(id) = doc.get_first(id_field).and_then(|v| v.as_str()) {
                scored.push((id.to_string(), score));
            }
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(limit as usize);
    let mut out = Vec::with_capacity(scored.len());
    for (id, _) in scored {
        if let Some(node) = materialized.nodes.get(&id) {
            let label = node_text(node, "label");
            out.push(SearchHit {
                node_id: id,
                excerpt: excerpt_of(label, node_text(node, "body")),
            });
        }
    }
    out
}

pub struct StatusReport {
    pub nodes: u64,
    pub edges: u64,
    pub events: u64,
    pub artifacts: u64,
}

/// Materialize + journal line count; artifacts = artifact-type nodes.
pub fn status(root: &Path) -> StatusReport {
    let Ok(journal) = Journal::open(root) else {
        return StatusReport {
            nodes: 0,
            edges: 0,
            events: 0,
            artifacts: 0,
        };
    };
    let Ok(entries) = journal.read_all() else {
        return StatusReport {
            nodes: 0,
            edges: 0,
            events: 0,
            artifacts: 0,
        };
    };
    let events_n = entries.len() as u64;
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
    let materialized = materialize(&events, None, true);
    let artifacts = materialized
        .nodes
        .values()
        .filter(|n| {
            n.get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t.eq_ignore_ascii_case("artifact"))
        })
        .count() as u64;
    StatusReport {
        nodes: materialized.nodes.len() as u64,
        edges: materialized.edges.len() as u64,
        events: events_n,
        artifacts,
    }
}

pub struct TimelineEntry {
    pub observed_utc: String,
    pub op: String,
    pub summary: String,
}

/// Journal order; filter = prefix `YYYY[-MM]`; summary = label or from->to.
pub fn timeline(root: &Path, filter: Option<&str>) -> Vec<TimelineEntry> {
    let Ok(journal) = Journal::open(root) else {
        return Vec::new();
    };
    let Ok(entries) = journal.read_all() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        if let Some(f) = filter {
            if !f.is_empty() && !e.observed_utc.starts_with(f) {
                continue;
            }
        }
        let summary = match e.op.as_str() {
            "node.upsert" => e
                .payload
                .get("label")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    e.payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default(),
            "edge.assert" | "edge.retract" => {
                match (
                    e.payload.get("from").and_then(|v| v.as_str()),
                    e.payload.get("to").and_then(|v| v.as_str()),
                ) {
                    (Some(from), Some(to)) => format!("{from}->{to}"),
                    _ => String::new(),
                }
            }
            _ => e
                .payload
                .get("key")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_default(),
        };
        out.push(TimelineEntry {
            observed_utc: e.observed_utc,
            op: e.op,
            summary,
        });
    }
    out
}

/// Project page render (Task 10).
///
/// Groups `BELONGS_TO` members whose `to` == `id` by member node `type`
/// (case-insensitive `decision`/`task`/`experiment`/`dataset`; other kinds
/// ignored) and lists member labels as `- ` bullets under `## Decisions` /
/// `## Tasks` / `## Experiments` / `## Datasets` (always emitted, in that
/// order, members sorted for determinism; missing/empty label falls back to
/// the member id). Retracted edges are skipped; validity windows are ignored
/// (`materialize(..., include_expired = true)`, same as `search`/`status`).
/// Unknown id (no node with that id, including unreadable journal) errors.
pub fn project_render(root: &Path, id: &str) -> Result<String, String> {
    let materialized = match Journal::open(root).and_then(|j| {
        j.read_all().map(|entries| {
            entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "op": e.op,
                        "payload": e.payload,
                        "observed_utc": e.observed_utc,
                    })
                })
                .collect::<Vec<_>>()
        })
    }) {
        Ok(events) => materialize(&events, None, true),
        Err(_) => materialize(&[], None, true),
    };
    let project = materialized
        .nodes
        .get(id)
        .ok_or_else(|| format!("unknown project: {id}"))?;
    let project_label = project
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    let mut decisions = Vec::new();
    let mut tasks = Vec::new();
    let mut experiments = Vec::new();
    let mut datasets = Vec::new();
    for edge in &materialized.edges {
        if edge.retracted || edge.to != id || edge.edge != crate::graph::EdgeType::BelongsTo {
            continue;
        }
        let Some(node) = materialized.nodes.get(&edge.from) else {
            continue;
        };
        let label = node
            .get("label")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(edge.from.as_str())
            .to_string();
        match node
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "decision" => decisions.push(label),
            "task" => tasks.push(label),
            "experiment" => experiments.push(label),
            "dataset" => datasets.push(label),
            _ => {}
        }
    }
    decisions.sort();
    tasks.sort();
    experiments.sort();
    datasets.sort();
    let mut out = format!("# {project_label}\n\n");
    let sections = [
        ("## Decisions", &decisions),
        ("## Tasks", &tasks),
        ("## Experiments", &experiments),
        ("## Datasets", &datasets),
    ];
    for (i, (header, members)) in sections.iter().enumerate() {
        out.push_str(header);
        out.push('\n');
        for m in members.iter() {
            out.push_str("- ");
            out.push_str(m);
            out.push('\n');
        }
        if i + 1 < sections.len() {
            out.push('\n');
        }
    }
    Ok(out)
}

/// Minimal flat TOML string reader for `profile/profile.toml` (Task 10).
///
/// Local 10-line reader reusing the `config.rs` hand-parser pattern (`#`
/// comments outside double quotes, double-quoted values with `\\` / `\"`
/// escapes only, bare interior quotes rejected): flat `key = "value"` lines
/// only, section headers and non-string values ignored, last duplicate wins.
/// Only `title`/`blurb`/`status` are kept; every other key is ignored without
/// error. Missing keys render as empty strings.
fn profile_string_map(text: &str) -> std::collections::HashMap<String, String> {
    fn strip_comment(line: &str) -> &str {
        let bytes = line.as_bytes();
        let mut in_quotes = false;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if in_quotes => {
                    i += 2;
                    continue;
                }
                b'"' => in_quotes = !in_quotes,
                b'#' if !in_quotes => return line[..i].trim_end(),
                _ => {}
            }
            i += 1;
        }
        line
    }
    fn unquote(value: &str) -> Option<String> {
        let inner = value.strip_prefix('"')?.strip_suffix('"')?;
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next()? {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    _ => return None,
                }
            } else if c == '"' {
                return None;
            } else {
                out.push(c);
            }
        }
        Some(out)
    }
    let mut map = std::collections::HashMap::new();
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();
        if !matches!(key, "title" | "blurb" | "status") {
            continue;
        }
        if let Some(s) = unquote(value) {
            map.insert(key.to_string(), s);
        }
    }
    map
}

/// Profile page render (Task 10).
///
/// Reads `<root>/profile/profile.toml` via [`profile_string_map`] and renders
/// byte-exact `"# {title}\n\n{blurb}\n\nstatus: {status}\n"` (absent keys are
/// empty strings). A missing/unreadable file errors with prefix
/// `"missing profile/profile.toml"`.
pub fn profile_render(root: &Path) -> Result<String, String> {
    let path = root.join("profile/profile.toml");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("missing profile/profile.toml: {e}"))?;
    let map = profile_string_map(&text);
    let title = map.get("title").map(String::as_str).unwrap_or("");
    let blurb = map.get("blurb").map(String::as_str).unwrap_or("");
    let status = map.get("status").map(String::as_str).unwrap_or("");
    Ok(format!("# {title}\n\n{blurb}\n\nstatus: {status}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    fn project_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "t:1", "type": "Task", "label": "task one"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "d:1", "type": "Decision", "label": "decision one"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "e:1", "type": "Experiment", "label": "experiment one"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "ds:1", "type": "Dataset", "label": "dataset one"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "u:1", "type": "Task", "label": "unrelated chores"}),
        )
        .unwrap();
        for from in ["t:1", "d:1", "e:1", "ds:1"] {
            j.append(
                "edge.assert",
                &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn guide_mentions_query_first() {
        let text = guide_text();
        assert!(text.contains("query"));
        assert!(text.find("query") < text.find("status"));
    }
    #[test]
    fn search_is_lexical_only() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "a:1", "type": "Task", "label": "SFT tokenizer"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "b:2", "type": "Task", "label": "unrelated chores"}),
        )
        .unwrap();
        j.append(
            "edge.assert",
            &serde_json::json!({"from": "b:2", "type": "FOLLOWS_UP", "to": "a:1"}),
        )
        .unwrap();
        let hits = search(dir.path(), "SFT", 20);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, "a:1");
    }
    #[test]
    fn status_counts_match_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        for id in ["a:1", "a:2", "a:3"] {
            j.append(
                "node.upsert",
                &serde_json::json!({"id": id, "type": "Task", "label": id}),
            )
            .unwrap();
        }
        j.append(
            "edge.assert",
            &serde_json::json!({"from": "a:1", "type": "FOLLOWS_UP", "to": "a:2"}),
        )
        .unwrap();
        let s = status(dir.path());
        assert_eq!((s.nodes, s.edges, s.events, s.artifacts), (3, 1, 4, 0));
    }
    #[test]
    fn search_empty_keyword_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "a:1", "type": "Task", "label": "hello world"}),
        )
        .unwrap();
        assert!(search(dir.path(), "", 20).is_empty());
    }
    #[test]
    fn search_limit_zero_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "a:1", "type": "Task", "label": "hello world"}),
        )
        .unwrap();
        assert!(search(dir.path(), "hello", 0).is_empty());
    }
    #[test]
    fn search_unparsable_query_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "a:1", "type": "Task", "label": "hello world"}),
        )
        .unwrap();
        // Gibberish punctuation must not panic; parity-read degrades to empty.
        assert!(search(dir.path(), "!!!???,,,", 20).is_empty());
    }
    #[test]
    fn timeline_filters_by_month() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "a:1", "type": "Task", "label": "aug"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "s:1", "type": "Task", "label": "sep"}),
        )
        .unwrap();
        // No Journal API supports backdating: `append` stamps wall-clock
        // observed_utc and ignores any payload observed_utc, so both rows
        // would share today's month. Rewrite the stored envelope timestamps
        // to give the filter two distinct months; assertions below are
        // unchanged in intent.
        let jpath = dir.path().join(".innen/journal.jsonl");
        let content = std::fs::read_to_string(&jpath).unwrap();
        let mut lines: Vec<serde_json::Value> = content
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        lines[0]["observed_utc"] = serde_json::json!("2026-08-01T00:00:00Z");
        lines[1]["observed_utc"] = serde_json::json!("2026-09-01T00:00:00Z");
        let out = lines
            .iter()
            .map(|v| serde_json::to_string(v).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&jpath, out).unwrap();
        let rows = timeline(dir.path(), Some("2026-09"));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].observed_utc.starts_with("2026-09"));
    }
    #[test]
    fn project_render_groups_by_kind() {
        let dir = project_fixture();
        let out = project_render(dir.path(), "p:x").unwrap();
        for section in ["## Decisions", "## Tasks", "## Experiments", "## Datasets"] {
            assert!(out.contains(section), "missing {section}");
        }
        assert!(!out.contains("unrelated"));
    }
    #[test]
    fn project_unknown_id_errors() {
        let dir = project_fixture();
        let err = project_render(dir.path(), "p:nope").unwrap_err();
        assert_eq!(err, "unknown project: p:nope");
    }
    #[test]
    fn profile_renders_fixture_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        std::fs::write(
            dir.path().join("profile/profile.toml"),
            "title = \"T\"\nblurb = \"B\"\nstatus = \"active\"\nextra = \"ignored\"\n",
        )
        .unwrap();
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# T\n\nB\n\nstatus: active\n");
    }
    #[test]
    fn profile_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = profile_render(dir.path()).unwrap_err();
        assert!(
            err.starts_with("missing profile/profile.toml"),
            "got: {err}"
        );
    }
}

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
     What remains? project [id] returns compact recorded tasks across projects or one project.\n\
     Need evidence? project <id> --view evidence; legacy project page: --view full.\n\
     Find prior knowledge: query --q <term>. Continue a conversation: resume <session-id>.\n\
     Trace original sources: wiki sync after wiki edits, then trace --q <clue>; use returned expand_argv for exact records.\n\
     Task status is recorded evidence; unknown status and heuristic conversation candidates are not confirmed unfinished work."
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
    let resolved = crate::graph::resolve_project_id(&materialized, id)?;
    let (target_id, project) = materialized
        .nodes
        .get_key_value(&resolved)
        .map(|(key, value)| (key.as_str(), value))
        .expect("resolved project exists");
    let project_label = project
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(target_id);
    let mut decisions = Vec::new();
    let mut tasks = Vec::new();
    let mut experiments = Vec::new();
    let mut datasets = Vec::new();
    for edge in &materialized.edges {
        if edge.retracted || edge.to != target_id || edge.edge != crate::graph::EdgeType::BelongsTo
        {
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
    fn project_render_resolves_shorthand_id() {
        let dir = project_fixture();
        // Fixture defines "p:x"; passing shorthand "x" should resolve to "p:x".
        let out = project_render(dir.path(), "x").unwrap();
        assert!(out.contains("# Project X"));
        assert!(out.contains("## Tasks"));
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
    #[test]
    fn profile_escapes_quote_and_backslash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        // TOML bytes: title = "a\"b\\c" -> value a"b\c
        std::fs::write(
            dir.path().join("profile/profile.toml"),
            "title = \"a\\\"b\\\\c\"\nblurb = \"B\"\nstatus = \"s\"\n",
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("profile/profile.toml")).unwrap();
        let map = profile_string_map(&text);
        assert_eq!(map.get("title").map(String::as_str), Some("a\"b\\c"));
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# a\"b\\c\n\nB\n\nstatus: s\n");
    }
    #[test]
    fn profile_hash_inside_quotes_kept_outside_stripped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        std::fs::write(
            dir.path().join("profile/profile.toml"),
            "# leading comment\ntitle = \"a#b\" # trailing comment\nblurb = \"B\"#no-space\nstatus = \"ok\"\n",
        )
        .unwrap();
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# a#b\n\nB\n\nstatus: ok\n");
    }
    #[test]
    fn profile_other_section_header_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        std::fs::write(
            dir.path().join("profile/profile.toml"),
            "title = \"T\"\n[other]\nblurb = \"B\"\nstatus = \"active\"\nfoo = \"bar\"\n",
        )
        .unwrap();
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# T\n\nB\n\nstatus: active\n");
    }
    #[test]
    fn profile_duplicate_keys_last_wins_bare_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        std::fs::write(
            dir.path().join("profile/profile.toml"),
            "title = \"first\"\ntitle = \"second\"\nblurb = \"B\"\nstatus = \"s\"\ncount = 3\n",
        )
        .unwrap();
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# second\n\nB\n\nstatus: s\n");
    }
    #[test]
    fn profile_missing_keys_render_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        std::fs::write(dir.path().join("profile/profile.toml"), "title = \"T\"\n").unwrap();
        let out = profile_render(dir.path()).unwrap();
        assert_eq!(out, "# T\n\n\n\nstatus: \n");
    }
    #[test]
    fn project_retracted_belongs_to_skipped() {
        let dir = project_fixture();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "edge.retract",
            &serde_json::json!({"from": "t:1", "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
        let out = project_render(dir.path(), "p:x").unwrap();
        assert!(!out.contains("task one"), "retracted member skipped: {out}");
        assert!(out.contains("decision one"));
    }
    #[test]
    fn project_label_missing_falls_back_to_id() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "t:nolabel", "type": "Task"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "t:empty", "type": "Task", "label": ""}),
        )
        .unwrap();
        for from in ["t:nolabel", "t:empty"] {
            j.append(
                "edge.assert",
                &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
            )
            .unwrap();
        }
        let out = project_render(dir.path(), "p:x").unwrap();
        assert!(
            out.contains("- t:nolabel"),
            "missing label falls back to id: {out}"
        );
        assert!(
            out.contains("- t:empty"),
            "empty label falls back to id: {out}"
        );
    }
    #[test]
    fn project_members_sorted_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "t:z", "type": "Task", "label": "zeta"}),
        )
        .unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "t:a", "type": "Task", "label": "alpha"}),
        )
        .unwrap();
        for from in ["t:z", "t:a"] {
            j.append(
                "edge.assert",
                &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
            )
            .unwrap();
        }
        let out = project_render(dir.path(), "p:x").unwrap();
        let pos_alpha = out.find("alpha").expect("alpha present");
        let pos_zeta = out.find("zeta").expect("zeta present");
        assert!(pos_alpha < pos_zeta, "members sorted: {out}");
    }
    #[test]
    fn project_custom_task_counted_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
        )
        .unwrap();
        for (id, ty, label) in [
            ("t:lower", "task", "lower task"),
            ("t:upper", "TASK", "upper task"),
            ("t:mixed", "TaSk", "mixed task"),
        ] {
            j.append(
                "node.upsert",
                &serde_json::json!({"id": id, "type": ty, "label": label}),
            )
            .unwrap();
            j.append(
                "edge.assert",
                &serde_json::json!({"from": id, "type": "BELONGS_TO", "to": "p:x"}),
            )
            .unwrap();
        }
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "n:other", "type": "Note", "label": "ignored note"}),
        )
        .unwrap();
        j.append(
            "edge.assert",
            &serde_json::json!({"from": "n:other", "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
        let out = project_render(dir.path(), "p:x").unwrap();
        assert!(
            out.contains("- lower task"),
            "lowercase task counted: {out}"
        );
        assert!(
            out.contains("- upper task"),
            "uppercase task counted: {out}"
        );
        assert!(
            out.contains("- mixed task"),
            "mixed-case task counted: {out}"
        );
        assert!(
            !out.contains("ignored note"),
            "unknown kinds still ignored: {out}"
        );
    }
    #[test]
    fn project_empty_sections_still_emitted() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        j.append(
            "node.upsert",
            &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
        )
        .unwrap();
        let out = project_render(dir.path(), "p:x").unwrap();
        for section in ["## Decisions", "## Tasks", "## Experiments", "## Datasets"] {
            assert!(out.contains(section), "missing {section}: {out}");
        }
        assert_eq!(
            out,
            "# Project X\n\n## Decisions\n\n## Tasks\n\n## Experiments\n\n## Datasets\n"
        );
    }
}

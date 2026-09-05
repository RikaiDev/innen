//! One-time migration from legacy km repository to innen (Task 16, spec §7).
//!
//! `innen migrate <old> --out <new>`
//! - Old repository is strictly read-only; rollback = delete <new>.
//! - Idempotent: if <new> already has a valid migrated journal, verifies and reports.
//! - Closed 3-line mapping:
//!   - `upsert_node` -> `node.upsert`
//!   - `assert_edge` -> `edge.assert` (`recorded_at` -> `valid_from`)
//!   - `retract_edge` -> `edge.retract` (parses `edge_id` if `edge` object is missing)
//! - Timestamps with timezone offsets (e.g. `+08:00`) are canonicalized to UTC `Z`.
//! - Rebuilds derived indexes (`redb` + `tantivy`) on `<new>`.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

use crate::graph::is_uri_shape;
use crate::ids::event_id;
use crate::journal::{format_utc, observed_utc_now};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuarantinedEvent {
    pub old_event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RewrittenEvent {
    pub old_event_id: String,
    pub new_event_id: String,
    pub rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrateReport {
    pub migrated_events: u64,
    pub quarantined: Vec<QuarantinedEvent>,
    pub rewritten: Vec<RewrittenEvent>,
}

#[derive(Debug, Error)]
pub enum MigrateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("old repo not found at: {0}")]
    OldNotFound(PathBuf),
    #[error("journal error: {0}")]
    Journal(#[from] crate::journal::JournalError),
    #[error("index error: {0}")]
    Index(#[from] crate::index::IndexError),
}

/// Convert civil date to days since Unix epoch (1970-01-01).
fn civil_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parse RFC3339 string (including timezone offsets like +08:00) to canonical UTC YYYY-MM-DDTHH:MM:SSZ.
pub fn parse_rfc3339_to_utc(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() == 20 && s.ends_with('Z') && &s[10..11] == "T" {
        return Some(s.to_string());
    }
    if s.len() == 10 && s.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Some(format!("{s}T00:00:00Z"));
    }
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;

    let days = civil_to_days(year, month, day);
    let mut total_secs = days * 86400 + hour * 3600 + min * 60 + sec;

    if s.len() >= 25 {
        let sign_char = s.chars().nth(19)?;
        let off_h: i64 = s.get(20..22)?.parse().ok()?;
        let off_m: i64 = s.get(23..25)?.parse().ok()?;
        let off_secs = off_h * 3600 + off_m * 60;
        match sign_char {
            '+' => total_secs -= off_secs,
            '-' => total_secs += off_secs,
            _ => return None,
        }
    } else if s.len() > 19 && !s.ends_with('Z') {
        return None;
    }

    if total_secs < 0 {
        return None;
    }
    Some(format_utc(total_secs as u64))
}

/// Remap old node types to canonical NodeType names per spec §3.
pub fn remap_node_type(raw: &str) -> (&str, bool) {
    match raw.to_ascii_lowercase().as_str() {
        "meeting" => ("Conversation", true),
        "workstream" => ("Task", true),
        "data-session" => ("Dataset", true),
        "harvest-gap" => ("Task", true),
        "cloud-blob" => ("Artifact", true),
        "workspace" => ("Project", true),
        "source" => ("Artifact", true),
        "capability" => ("Artifact", true),
        "product" => ("Product", raw != "Product"),
        "experiment" => ("Experiment", raw != "Experiment"),
        "task" => ("Task", raw != "Task"),
        "artifact" => ("Artifact", raw != "Artifact"),
        "conversation" => ("Conversation", raw != "Conversation"),
        "project" => ("Project", raw != "Project"),
        "decision" => ("Decision", raw != "Decision"),
        "dataset" => ("Dataset", raw != "Dataset"),
        "model" => ("Model", raw != "Model"),
        _ => (raw, false),
    }
}

/// Infer canonical NodeType from an ID prefix (e.g. `project:foo` -> `Project`).
pub fn infer_type_from_id(id: &str) -> &'static str {
    let prefix = id.split(':').next().unwrap_or("").to_ascii_lowercase();
    match prefix.as_str() {
        "project" | "p" => "Project",
        "artifact" | "a" => "Artifact",
        "task" | "t" | "workstream" => "Task",
        "decision" | "d" => "Decision",
        "dataset" => "Dataset",
        "model" => "Model",
        "conversation" | "c" | "meeting" => "Conversation",
        "experiment" => "Experiment",
        _ => "Artifact",
    }
}

/// Extract human-readable project label from legacy project markdown file if present.
pub fn extract_project_label(old_root: &Path, project_stem: &str) -> Option<String> {
    let p = old_root
        .join("04-index/projects")
        .join(format!("{project_stem}.md"));
    if !p.exists() {
        return None;
    }
    let content = fs::read_to_string(p).ok()?;
    let mut in_frontmatter = false;
    let mut title_heading = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter {
            if let Some(val) = trimmed.strip_prefix("project:") {
                let v = val.trim().trim_matches('"').trim_matches('\'').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        } else if trimmed.starts_with("# ") && title_heading.is_none() {
            let h = trimmed.strip_prefix("# ").unwrap().trim();
            let clean = h.strip_suffix("專案總帳").unwrap_or(h).trim();
            if !clean.is_empty() {
                title_heading = Some(clean.to_string());
            }
        }
    }
    title_heading
}

/// Convert one raw legacy event into an innen event tuple:
/// `(op, observed_utc, payload, rewritten_records)`
pub fn migrate_event(
    raw: &Value,
) -> Result<(String, String, Value, Vec<RewrittenEvent>), QuarantinedEvent> {
    let obj = raw.as_object().ok_or_else(|| QuarantinedEvent {
        old_event_id: "unknown".to_string(),
        reason: "line is not a JSON object".to_string(),
    })?;

    let old_id = obj
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("missing-event-id")
        .to_string();

    let op = obj
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| QuarantinedEvent {
            old_event_id: old_id.clone(),
            reason: "missing op field".to_string(),
        })?;

    let recorded_at = obj
        .get("recorded_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let observed_utc = parse_rfc3339_to_utc(recorded_at).unwrap_or_else(observed_utc_now);

    let mut rewritten = Vec::new();

    match op {
        "upsert_node" => {
            let node_val = obj.get("node").ok_or_else(|| QuarantinedEvent {
                old_event_id: old_id.clone(),
                reason: "upsert_node missing node object".to_string(),
            })?;
            let mut node_obj = node_val
                .as_object()
                .cloned()
                .ok_or_else(|| QuarantinedEvent {
                    old_event_id: old_id.clone(),
                    reason: "node is not an object".to_string(),
                })?;

            let nid = node_obj
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| QuarantinedEvent {
                    old_event_id: old_id.clone(),
                    reason: "node missing string id".to_string(),
                })?
                .to_string();

            let node_type_opt = node_obj
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            match node_type_opt {
                None => {
                    let inferred = nid.split(':').next().unwrap_or("node");
                    let (mapped, _) = remap_node_type(inferred);
                    node_obj.insert("type".to_string(), json!(mapped));
                    rewritten.push(RewrittenEvent {
                        old_event_id: old_id.clone(),
                        new_event_id: String::new(),
                        rule: "infer_type_from_id_prefix".to_string(),
                    });
                }
                Some(t) => {
                    let (mapped, changed) = remap_node_type(&t);
                    if changed {
                        node_obj.insert("type".to_string(), json!(mapped));
                        rewritten.push(RewrittenEvent {
                            old_event_id: old_id.clone(),
                            new_event_id: String::new(),
                            rule: format!("remapped_type_{t}_to_{mapped}"),
                        });
                    }
                }
            }

            let payload = Value::Object(node_obj);
            let new_id = event_id("node.upsert", &payload);
            for r in &mut rewritten {
                r.new_event_id = new_id.clone();
            }

            Ok(("node.upsert".to_string(), observed_utc, payload, rewritten))
        }
        "assert_edge" => {
            let edge_val = obj.get("edge").ok_or_else(|| QuarantinedEvent {
                old_event_id: old_id.clone(),
                reason: "assert_edge missing edge object".to_string(),
            })?;
            let mut edge_obj = edge_val
                .as_object()
                .cloned()
                .ok_or_else(|| QuarantinedEvent {
                    old_event_id: old_id.clone(),
                    reason: "edge is not an object".to_string(),
                })?;

            let from = edge_obj.get("from").and_then(|v| v.as_str());
            let to = edge_obj.get("to").and_then(|v| v.as_str());
            let etype = edge_obj.get("type").and_then(|v| v.as_str());

            if from.is_none() || to.is_none() || etype.is_none() {
                return Err(QuarantinedEvent {
                    old_event_id: old_id,
                    reason: "edge missing from/to/type".to_string(),
                });
            }

            edge_obj.insert("valid_from".to_string(), json!(observed_utc));
            rewritten.push(RewrittenEvent {
                old_event_id: old_id.clone(),
                new_event_id: String::new(),
                rule: "recorded_at_to_valid_from".to_string(),
            });

            let payload = Value::Object(edge_obj);
            let new_id = event_id("edge.assert", &payload);
            for r in &mut rewritten {
                r.new_event_id = new_id.clone();
            }

            Ok(("edge.assert".to_string(), observed_utc, payload, rewritten))
        }
        "retract_edge" => {
            let mut edge_obj = obj
                .get("edge")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();

            let has_parties = edge_obj.contains_key("from")
                && edge_obj.contains_key("to")
                && edge_obj.contains_key("type");

            if !has_parties {
                let edge_id_str = obj.get("edge_id").and_then(|v| v.as_str()).ok_or_else(|| {
                    QuarantinedEvent {
                        old_event_id: old_id.clone(),
                        reason: "retract_edge missing both edge object and edge_id".to_string(),
                    }
                })?;

                let parts: Vec<&str> = edge_id_str.split('|').collect();
                if parts.len() != 3 {
                    return Err(QuarantinedEvent {
                        old_event_id: old_id,
                        reason: format!("malformed edge_id: {edge_id_str}"),
                    });
                }

                edge_obj.insert("from".to_string(), json!(parts[0]));
                edge_obj.insert("type".to_string(), json!(parts[1]));
                edge_obj.insert("to".to_string(), json!(parts[2]));

                rewritten.push(RewrittenEvent {
                    old_event_id: old_id.clone(),
                    new_event_id: String::new(),
                    rule: "parse_edge_from_edge_id".to_string(),
                });
            }

            if let Some(ev) = obj.get("evidence").and_then(|v| v.as_str()) {
                if !edge_obj.contains_key("evidence") {
                    edge_obj.insert("evidence".to_string(), json!(ev));
                }
            }

            let payload = Value::Object(edge_obj);
            let new_id = event_id("edge.retract", &payload);
            for r in &mut rewritten {
                r.new_event_id = new_id.clone();
            }

            Ok(("edge.retract".to_string(), observed_utc, payload, rewritten))
        }
        _ => Err(QuarantinedEvent {
            old_event_id: old_id,
            reason: format!("unknown op: {op}"),
        }),
    }
}

/// Recursively copy directory contents.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let dst_child = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_all(&entry.path(), &dst_child)?;
        } else {
            fs::copy(entry.path(), dst_child)?;
        }
    }
    Ok(())
}

/// Execute one-time migration from `old_root` to `new_root`.
pub fn migrate(old_root: &Path, new_root: &Path) -> Result<MigrateReport, MigrateError> {
    if !old_root.exists() {
        return Err(MigrateError::OldNotFound(old_root.to_path_buf()));
    }

    let target_journal = new_root.join(".innen/journal.jsonl");

    // Idempotent: if target journal exists, scan and report without mutating.
    if target_journal.exists() {
        let mut count: u64 = 0;
        let reader = BufReader::new(File::open(&target_journal)?);
        for line in reader.lines() {
            let line = line?;
            if !line.trim().is_empty() {
                count += 1;
            }
        }
        return Ok(MigrateReport {
            migrated_events: count.saturating_sub(1), // subtract schema stamp
            quarantined: Vec::new(),
            rewritten: Vec::new(),
        });
    }

    fs::create_dir_all(new_root.join(".innen"))?;

    // Copy standard KB directories if they exist
    for dir_name in ["01-raw", "02-wiki", "03-output", "04-index"] {
        let src_dir = old_root.join(dir_name);
        if src_dir.exists() {
            copy_dir_all(&src_dir, &new_root.join(dir_name))?;
        }
    }

    // Copy top-level Markdown files
    if let Ok(entries) = fs::read_dir(old_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" || ext == "toml" {
                        let _ = fs::copy(&path, new_root.join(entry.file_name()));
                    }
                }
            }
        }
    }

    let legacy_events_file = old_root.join("04-index/graph/events.jsonl");
    let mut migrated_events = 0;
    let mut quarantined = Vec::new();
    let mut rewritten = Vec::new();

    let mut raw_migrated = Vec::new();
    let mut known_nodes: HashSet<String> = HashSet::new();
    let mut referenced_endpoints: Vec<(String, String)> = Vec::new();

    if legacy_events_file.exists() {
        let reader = BufReader::new(File::open(legacy_events_file)?);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let raw: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    quarantined.push(QuarantinedEvent {
                        old_event_id: "unparseable".to_string(),
                        reason: format!("JSON error: {e}"),
                    });
                    continue;
                }
            };

            match migrate_event(&raw) {
                Ok((op, observed_utc, payload, rewrites)) => {
                    if op == "node.upsert" {
                        if let Some(nid) = payload.get("id").and_then(|v| v.as_str()) {
                            known_nodes.insert(nid.to_string());
                        }
                    } else if op == "edge.assert" || op == "edge.retract" {
                        if let Some(from) = payload.get("from").and_then(|v| v.as_str()) {
                            referenced_endpoints.push((from.to_string(), observed_utc.clone()));
                        }
                        if let Some(to) = payload.get("to").and_then(|v| v.as_str()) {
                            referenced_endpoints.push((to.to_string(), observed_utc.clone()));
                        }
                    }
                    raw_migrated.push((op, observed_utc, payload, rewrites));
                }
                Err(q) => {
                    quarantined.push(q);
                }
            }
        }
    }

    // Synthesize nodes from project ledgers (04-index/projects/*.md) if not already present
    let mut synthesized_nodes: Vec<(String, String, Value, RewrittenEvent)> = Vec::new();
    let projects_dir = old_root.join("04-index/projects");
    if projects_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&projects_dir) {
            let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                if path.is_file() {
                    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                        if file_name.ends_with(".md") && !file_name.starts_with('_') {
                            let stem = &file_name[..file_name.len() - 3];
                            let candidate1 = format!("project:{stem}");
                            let candidate2 = format!("p:{stem}");
                            if !known_nodes.contains(&candidate1)
                                && !known_nodes.contains(&candidate2)
                            {
                                let label = extract_project_label(old_root, stem)
                                    .unwrap_or_else(|| stem.to_string());
                                let node_id = candidate1;
                                let payload = json!({
                                    "id": node_id,
                                    "type": "Project",
                                    "label": label,
                                });
                                let eid = event_id("node.upsert", &payload);
                                let observed_utc = "2026-01-01T00:00:00Z".to_string();
                                synthesized_nodes.push((
                                    "node.upsert".to_string(),
                                    observed_utc,
                                    payload,
                                    RewrittenEvent {
                                        old_event_id: format!("legacy_project_file:{file_name}"),
                                        new_event_id: eid,
                                        rule: "synthesize_project_from_ledger".to_string(),
                                    },
                                ));
                                known_nodes.insert(node_id);
                            }
                        }
                    }
                }
            }
        }
    }

    // Synthesize any remaining referenced missing endpoints (e.g. missing artifacts)
    for (endpoint, ts) in referenced_endpoints {
        if !known_nodes.contains(&endpoint) && !is_uri_shape(&endpoint) {
            let ntype = infer_type_from_id(&endpoint);
            let label = if ntype == "Project" {
                let stem = endpoint
                    .strip_prefix("project:")
                    .or_else(|| endpoint.strip_prefix("p:"))
                    .unwrap_or(&endpoint);
                extract_project_label(old_root, stem).unwrap_or_else(|| stem.to_string())
            } else {
                endpoint.split(':').nth(1).unwrap_or(&endpoint).to_string()
            };
            let payload = json!({
                "id": endpoint,
                "type": ntype,
                "label": label,
            });
            let eid = event_id("node.upsert", &payload);
            synthesized_nodes.push((
                "node.upsert".to_string(),
                ts,
                payload,
                RewrittenEvent {
                    old_event_id: format!("referenced_endpoint:{endpoint}"),
                    new_event_id: eid,
                    rule: "synthesize_missing_referenced_node".to_string(),
                },
            ));
            known_nodes.insert(endpoint);
        }
    }

    let mut journal_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&target_journal)?;

    // Write schema stamp as the very first line
    let stamp = json!({
        "id": "schema:innen/v1",
        "observed_utc": observed_utc_now(),
        "op": "schema.stamp",
        "payload": {
            "version": "innen/v1",
            "migrated_from": "llm-km"
        }
    });
    writeln!(journal_file, "{stamp}")?;

    // Write synthesized nodes first
    for (op, observed_utc, payload, rw) in synthesized_nodes {
        let id = event_id(&op, &payload);
        let mut stored = Map::new();
        stored.insert("id".to_string(), json!(id));
        stored.insert("observed_utc".to_string(), json!(observed_utc));
        stored.insert("op".to_string(), json!(op));
        stored.insert("payload".to_string(), payload);

        writeln!(journal_file, "{}", Value::Object(stored))?;
        migrated_events += 1;
        rewritten.push(rw);
    }

    // Write all migrated events
    for (op, observed_utc, payload, rewrites) in raw_migrated {
        let id = event_id(&op, &payload);
        let mut stored = Map::new();
        stored.insert("id".to_string(), json!(id));
        stored.insert("observed_utc".to_string(), json!(observed_utc));
        stored.insert("op".to_string(), json!(op));
        stored.insert("payload".to_string(), payload);

        writeln!(journal_file, "{}", Value::Object(stored))?;
        migrated_events += 1;
        rewritten.extend(rewrites);
    }

    journal_file.flush()?;

    // Rebuild derived indexes (redb + tantivy)
    crate::index::rebuild(new_root)?;

    Ok(MigrateReport {
        migrated_events,
        quarantined,
        rewritten,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_rfc3339_converts_offset_to_utc_z() {
        let utc = parse_rfc3339_to_utc("2026-09-02T22:30:00+08:00").unwrap();
        assert_eq!(utc, "2026-09-02T14:30:00Z");

        let utc_day = parse_rfc3339_to_utc("2026-09-02").unwrap();
        assert_eq!(utc_day, "2026-09-02T00:00:00Z");

        let utc_z = parse_rfc3339_to_utc("2026-09-02T14:30:00Z").unwrap();
        assert_eq!(utc_z, "2026-09-02T14:30:00Z");
    }

    #[test]
    fn node_upsert_remaps_legacy_types() {
        let raw = json!({
            "event_id": "e:1",
            "op": "upsert_node",
            "recorded_at": "2026-09-02T22:30:00+08:00",
            "node": {
                "id": "meeting:1",
                "type": "meeting",
                "label": "Daily Sync"
            }
        });
        let (op, observed, payload, rewritten) = migrate_event(&raw).unwrap();
        assert_eq!(op, "node.upsert");
        assert_eq!(observed, "2026-09-02T14:30:00Z");
        assert_eq!(
            payload.get("type").and_then(|v| v.as_str()),
            Some("Conversation")
        );
        assert_eq!(rewritten.len(), 1);
        assert_eq!(rewritten[0].rule, "remapped_type_meeting_to_Conversation");
    }

    #[test]
    fn node_upsert_infers_type_from_id() {
        let raw = json!({
            "event_id": "e:2",
            "op": "upsert_node",
            "recorded_at": "2026-09-02T22:30:00+08:00",
            "node": {
                "id": "project:nhri-pcne",
                "label": "PCNe"
            }
        });
        let (op, _, payload, rewritten) = migrate_event(&raw).unwrap();
        assert_eq!(op, "node.upsert");
        assert_eq!(
            payload.get("type").and_then(|v| v.as_str()),
            Some("Project")
        );
        assert_eq!(rewritten[0].rule, "infer_type_from_id_prefix");
    }

    #[test]
    fn edge_retract_parses_edge_id_fallback() {
        let raw = json!({
            "event_id": "e:3",
            "op": "retract_edge",
            "edge_id": "a:1|SAME_AS|b:2",
            "recorded_at": "2026-09-02T22:30:00+08:00"
        });
        let (op, _, payload, rewritten) = migrate_event(&raw).unwrap();
        assert_eq!(op, "edge.retract");
        assert_eq!(payload.get("from").and_then(|v| v.as_str()), Some("a:1"));
        assert_eq!(
            payload.get("type").and_then(|v| v.as_str()),
            Some("SAME_AS")
        );
        assert_eq!(payload.get("to").and_then(|v| v.as_str()), Some("b:2"));
        assert_eq!(rewritten[0].rule, "parse_edge_from_edge_id");
    }

    #[test]
    fn migration_end_to_end_and_idempotent() {
        let old_dir = tempdir().unwrap();
        let graph_dir = old_dir.path().join("04-index/graph");
        fs::create_dir_all(&graph_dir).unwrap();
        let events_file = graph_dir.join("events.jsonl");

        let e1 = json!({
            "event_id": "e1",
            "op": "upsert_node",
            "recorded_at": "2026-09-02T22:30:00+08:00",
            "node": {"id": "project:p1", "type": "project", "label": "P1"}
        });
        let e2 = json!({
            "event_id": "e2",
            "op": "assert_edge",
            "recorded_at": "2026-09-02T22:30:00+08:00",
            "edge": {"from": "task:t1", "to": "project:p1", "type": "BELONGS_TO"}
        });
        fs::write(&events_file, format!("{e1}\n{e2}\n")).unwrap();

        let new_dir = tempdir().unwrap();
        let report = migrate(old_dir.path(), new_dir.path()).unwrap();
        // 1 synthesized node (task:t1) + 2 raw events = 3 migrated events
        assert_eq!(report.migrated_events, 3);
        assert!(report.quarantined.is_empty());
        assert_eq!(report.rewritten.len(), 3);

        // Verify index rebuild created redb
        assert!(new_dir.path().join(".innen/index.redb").exists());

        // Verify doctor passes with exit code 0 on the migrated repo
        let doc_report = crate::doctor::run(new_dir.path());
        assert_eq!(
            doc_report.exit_code, 0,
            "doctor should pass on migrated repo: {doc_report:?}"
        );

        // Second run is idempotent
        let report2 = migrate(old_dir.path(), new_dir.path()).unwrap();
        assert_eq!(report2.migrated_events, 3);
    }

    #[test]
    fn extract_project_label_parses_frontmatter_and_heading() {
        let dir = tempdir().unwrap();
        let pdir = dir.path().join("04-index/projects");
        fs::create_dir_all(&pdir).unwrap();

        fs::write(
            pdir.join("p-alpha.md"),
            "---\nproject: Alpha Health Care\nstatus: active\n---\n# Alpha\n",
        )
        .unwrap();
        assert_eq!(
            extract_project_label(dir.path(), "p-alpha"),
            Some("Alpha Health Care".to_string())
        );

        fs::write(
            pdir.join("p-beta.md"),
            "# Beta Traffic Analytics 專案總帳\n\nBoundary details\n",
        )
        .unwrap();
        assert_eq!(
            extract_project_label(dir.path(), "p-beta"),
            Some("Beta Traffic Analytics".to_string())
        );
    }
}

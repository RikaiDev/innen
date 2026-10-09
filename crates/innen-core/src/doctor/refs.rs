//! `ref-integrity`: every live edge endpoint resolves.

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::journal::JournalEntry;

use super::paths::{is_not_found, journal_path};
use super::Check;

/// Uri shape, mirroring [`crate::graph::validate`]: non-empty and starting
/// with `/` or containing `://`.
fn is_uri(s: &str) -> bool {
    !s.is_empty() && (s.starts_with('/') || s.contains("://"))
}

/// `ref-integrity`: every non-retracted edge endpoint resolves.
///
/// `from` must be a known node id; `to` must be a known node id or a Uri.
/// Only lines that parse are considered (unparseable lines already fail
/// `journal-valid`); retracted rows are history, not live references.
pub(super) fn check_ref_integrity(root: &Path) -> Check {
    let name = "ref-integrity".to_string();
    let content = match fs::read_to_string(journal_path(root)) {
        Ok(content) => content,
        Err(e) if is_not_found(&e) => {
            return Check {
                name,
                ok: true,
                detail: "no edges (empty journal)".to_string(),
            };
        }
        Err(e) => {
            return Check {
                name,
                ok: false,
                detail: format!("journal unreadable: {e}"),
            };
        }
    };
    let mut events = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JournalEntry>(line) else {
            continue;
        };
        events.push(json!({
            "op": entry.op,
            "payload": entry.payload,
            "observed_utc": entry.observed_utc,
        }));
    }
    let materialized = crate::graph::materialize(&events, None, true);
    let mut dangling: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for edge in materialized.edges.iter().filter(|e| !e.retracted) {
        checked += 1;
        if !materialized.nodes.contains_key(&edge.from) {
            dangling.push(format!(
                "from '{}' -[{}]-> '{}' (unknown from)",
                edge.from, edge.edge, edge.to
            ));
            continue;
        }
        if !is_uri(&edge.to) && !materialized.nodes.contains_key(&edge.to) {
            dangling.push(format!(
                "from '{}' -[{}]-> '{}' (unknown to)",
                edge.from, edge.edge, edge.to
            ));
        }
    }
    if dangling.is_empty() {
        Check {
            name,
            ok: true,
            detail: format!("{checked} edges, all resolve"),
        }
    } else {
        let mut shown: String = dangling
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        if dangling.len() > 5 {
            shown += &format!("; and {} more", dangling.len() - 5);
        }
        Check {
            name,
            ok: false,
            detail: format!("{} dangling edge(s): {shown}", dangling.len()),
        }
    }
}

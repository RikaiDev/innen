//! Journal counts: materialized nodes, edges, events, and artifacts.

use std::path::Path;

use crate::graph::materialize;
use crate::journal::Journal;

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

//! Journal history in order, optionally filtered by a `YYYY[-MM]` prefix.

use std::path::Path;

use crate::journal::Journal;

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

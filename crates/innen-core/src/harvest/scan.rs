//! Inbox selection and event construction. [`check`] and [`ingest::run`] both
//! select through [`unconsumed`], so a dry run cannot disagree with the run it
//! previews.

use std::path::Path;

use crate::config::Config;
use crate::tap::Event;

use super::report::{HarvestReport, TapReport};
use super::tap::TAP_ID;

/// Recursively list files matching harvest patterns under inbox_path.
/// Missing/unreadable inbox → empty.
pub(super) fn list_inbox_files(root: &Path, config: &Config) -> Vec<String> {
    let inbox = root.join(&config.inbox_path);
    let mut names = Vec::new();
    
    // Walk the inbox directory recursively
    if !inbox.exists() {
        return Vec::new();
    }
    
    let walker = walkdir::WalkDir::new(&inbox).into_iter();
    
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        
        // Check if file matches any harvest pattern
        let rel = path.strip_prefix(&inbox).unwrap_or(path);
        let rel_str = rel.to_string_lossy();
        
        let matches = config.harvest_patterns.iter().any(|pattern| {
            // Simple glob matching: **/*.ext -> ends with .ext
            if pattern.starts_with("**/") {
                let suffix = &pattern[3..];
                // Handle **/*.ext pattern - extract extension
                if suffix.starts_with("*.") {
                    let ext = &suffix[1..]; // include the dot, e.g. ".jsonl"
                    rel_str.ends_with(ext)
                } else {
                    // Pattern like **/foo - match exact path suffix
                    rel_str.ends_with(suffix)
                }
            } else if pattern.starts_with("*.") {
                // Simple glob: *.md, *.json, etc.
                let ext = &pattern[1..]; // include the dot
                rel_str.ends_with(ext)
            } else if pattern.contains('*') {
                // Other glob patterns not supported yet
                false
            } else {
                rel_str == *pattern
            }
        });
        
        if matches {
            names.push(rel_str.into_owned());
        }
    }
    
    names.sort();
    names
}

fn sha8(rel: &str) -> String {
    crate::ids::sha256_hex(rel.as_bytes())[..8].to_string()
}

/// Inbox files this tap has not consumed, in listing order, plus the cursor
/// state that decided it.
///
/// [`check`] and [`ingest::run`] both select through here, so a dry run can
/// never disagree with the run it previews. A cursor that cannot name its
/// inputs (legacy count, unreadable file) contributes an empty consumed set:
/// re-listing everything is safe because the event id is content-derived and
/// the op is an upsert, whereas guessing a count would skip real files.
pub(super) fn unconsumed(root: &Path, config: &Config) -> (Vec<String>, crate::tap::CursorState) {
    let state = crate::tap::load_cursor(root, TAP_ID);
    let cursor = state
        .known()
        .map(|consumed| crate::tap::Cursor {
            consumed: consumed.clone(),
        })
        .unwrap_or_default();
    let files = list_inbox_files(root, config);
    let pending = cursor
        .unconsumed(&files)
        .into_iter()
        .cloned()
        .collect::<Vec<String>>();
    (pending, state)
}

/// The consumed set to persist after this run: everything already recorded,
/// plus what this run handled. Not pruned — a name that leaves and comes back
/// must stay consumed rather than be appended twice.
pub(super) fn advance(state: &crate::tap::CursorState, handled: &[String]) -> crate::tap::Cursor {
    let mut cursor = state
        .known()
        .map(|consumed| crate::tap::Cursor {
            consumed: consumed.clone(),
        })
        .unwrap_or_default();
    cursor.consumed.extend(handled.iter().cloned());
    cursor
}

fn source_id(rel: &str) -> String {
    format!("source:{}", sha8(rel))
}

pub(super) fn event_for_file(rel: &str, bytes: &[u8], text: &str) -> Event {
    let sha = crate::ids::sha256_hex(bytes);
    let payload = serde_json::json!({
        "id": source_id(rel),
        "type": crate::graph::NodeType::Artifact.to_string(),
        "label": rel,
        "body": text,
        "provenance": {
            "path": rel,
            "bytes": bytes.len() as u64,
            "sha256": sha,
            "observed_utc": crate::journal::observed_utc_now(),
        },
    });
    Event {
        op: "node.upsert".to_string(),
        payload,
        idempotency_key: sha,
    }
}

/// DRY-RUN: no appends, no cursor advance.
pub fn check(root: &Path, config: &Config) -> HarvestReport {
    let (pending, state) = unconsumed(root, config);
    let mut new_files = Vec::new();
    let mut skipped = Vec::new();
    let inbox = root.join(&config.inbox_path);
    for rel in pending {
        let text = std::fs::read_to_string(inbox.join(&rel)).unwrap_or_default();
        if crate::credentials::scan_credentials(&text).is_empty() {
            new_files.push(rel);
        } else {
            skipped.push(rel);
        }
    }
    HarvestReport {
        taps: vec![TapReport {
            id: TAP_ID.to_string(),
            new_files,
            skipped,
            cursor: state.label().to_string(),
        }],
    }
}

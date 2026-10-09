//! Inbox selection and event construction. [`check`] and [`ingest::run`] both
//! select through [`unconsumed`], so a dry run cannot disagree with the run it
//! previews.

use std::path::Path;

use crate::tap::Event;

use super::report::{HarvestReport, TapReport};
use super::tap::TAP_ID;

/// Sorted top-level `*.md` file names under `<root>/00-inbox/harvest`.
/// Missing/unreadable inbox → empty.
pub(super) fn list_inbox_files(root: &Path) -> Vec<String> {
    let inbox = root.join("00-inbox/harvest");
    let entries = match std::fs::read_dir(&inbox) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|o| o.to_str())
            .is_some_and(|e| e == "md");
        if !is_md {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
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
pub(super) fn unconsumed(root: &Path) -> (Vec<String>, crate::tap::CursorState) {
    let state = crate::tap::load_cursor(root, TAP_ID);
    let cursor = state
        .known()
        .map(|consumed| crate::tap::Cursor {
            consumed: consumed.clone(),
        })
        .unwrap_or_default();
    let files = list_inbox_files(root);
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
pub fn check(root: &Path) -> HarvestReport {
    let (pending, state) = unconsumed(root);
    let mut new_files = Vec::new();
    let mut skipped = Vec::new();
    let inbox = root.join("00-inbox/harvest");
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

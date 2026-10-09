//! Applying the harvest tap: append clean files, report credential hits.
//!
//! Selection lives in the parent module and is shared with `check`, so a dry
//! run can never disagree with the run it previews. This module owns only the
//! append side and what a skipped file reports.

use std::path::Path;

use crate::credentials::credential_preview;

use super::report::{IngestReport, Skipped};
use super::scan::{advance, event_for_file, unconsumed};
use super::tap::TAP_ID;

/// Append one `node.upsert` per clean file, skip credential hits
/// (reported, never appended), then record every consumed name —
/// added and skipped alike, so a credential hit is not retried forever
/// and a clean file is never skipped by a shifted cursor.
pub fn run(root: &Path) -> IngestReport {
    let (pending, state) = unconsumed(root);
    let total = pending.len();
    if total == 0 {
        return IngestReport {
            added: 0,
            skipped: Vec::new(),
        };
    }
    let Ok(journal) = crate::journal::Journal::open(root) else {
        return IngestReport {
            added: 0,
            skipped: Vec::new(),
        };
    };
    let inbox = root.join("00-inbox/harvest");
    let mut added: u64 = 0;
    let mut skipped = Vec::new();
    let mut processed: Vec<String> = Vec::with_capacity(total);
    for rel in &pending {
        let bytes = match std::fs::read(inbox.join(rel)) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let hits = crate::credentials::scan_credentials(&text);
        if let Some(pattern) = hits.first() {
            skipped.push(Skipped {
                path: rel.clone(),
                pattern: pattern.to_string(),
                preview: credential_preview(&text, pattern),
            });
            processed.push(rel.clone());
            continue;
        }
        let event = event_for_file(rel, &bytes, &text);
        match journal.append(&event.op, &event.payload) {
            Ok(_) => {
                added += 1;
                processed.push(rel.clone());
            }
            Err(_) => break,
        }
    }
    if !processed.is_empty() {
        let cursor = advance(&state, &processed);
        let _ = crate::tap::store_cursor(root, TAP_ID, &cursor);
    }
    IngestReport { added, skipped }
}

//! Phase 4: apply the prune decisions to `page.records` in place.
//!
//! A pruned record keeps its line and its provenance: it gains `pruned` and
//! `prune_reason`, and whichever of `preview` / `output` / `content` carried
//! the stale text is replaced with an omission note. Fields the source never
//! had are not invented, so a consumer can still tell a pruned record from a
//! record that merely had no preview.

use serde_json::{json, Value};

use super::super::Page;
use super::rules::PruneDecisions;

pub(super) struct ApplyOutcome {
    pub pruned_count: usize,
    pub saved_chars_estimate: usize,
}

pub(super) fn apply_prune_decisions(page: &mut Page, to_prune: &PruneDecisions) -> ApplyOutcome {
    let mut pruned_count = 0;
    let mut saved_chars_estimate = 0;

    for (idx, (reason, orig_chars)) in to_prune {
        let record = &mut page.records[*idx];
        let ev = &mut record.event;

        if let Some(obj) = ev.as_object_mut() {
            obj.insert("pruned".to_string(), json!(true));
            obj.insert("prune_reason".to_string(), json!(reason));

            if obj.contains_key("preview") {
                let note = format!("[omitted: {reason}]");
                let note_len = note.chars().count();
                obj.insert("preview".to_string(), json!(note));
                obj.insert("preview_tail".to_string(), json!(""));
                obj.insert("preview_truncated".to_string(), json!(false));
                obj.insert("chars".to_string(), json!(note_len));
                obj.insert("omitted_chars".to_string(), json!(orig_chars));
            }
            if obj.contains_key("output") {
                let note = format!("[tool output omitted: {reason}]");
                obj.insert("output".to_string(), json!(note));
            }
            if obj.contains_key("content") {
                let note = format!("[tool output omitted: {reason}]");
                obj.insert("content".to_string(), json!(note));
            }
            if let Some(payload) = obj.get_mut("payload").and_then(Value::as_object_mut) {
                if payload.contains_key("output") {
                    payload.insert(
                        "output".to_string(),
                        json!(format!("[tool output omitted: {reason}]")),
                    );
                }
            }
        }

        pruned_count += 1;
        saved_chars_estimate += orig_chars;
    }

    ApplyOutcome {
        pruned_count,
        saved_chars_estimate,
    }
}

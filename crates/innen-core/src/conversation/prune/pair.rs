//! Phase 2: link each tool call to the tool output it produced.
//!
//! Dialects disagree on how a call and its output are tied together, so the
//! match is tried in descending order of confidence: an explicit call id, the
//! recorded call line, then adjacency. Each output is claimed at most once, so
// one output never backs two calls.

use std::collections::BTreeSet;

use super::classify::ToolAction;
use super::scan::{CallMeta, OutputMeta};

pub(super) struct PairedTool {
    pub call_idx: usize,
    pub output_idx: usize,
    pub output_line: usize,
    pub action: ToolAction,
    pub is_error: bool,
    pub content_preview: String,
    pub original_chars: usize,
}

pub(super) fn pair_calls_with_outputs(
    calls: &[CallMeta],
    outputs: &[OutputMeta],
) -> Vec<PairedTool> {
    let mut paired: Vec<PairedTool> = Vec::new();
    let mut matched_outputs = BTreeSet::new();

    for call in calls {
        let matching_out = outputs.iter().find(|o| {
            if matched_outputs.contains(&o.record_idx) {
                return false;
            }
            if let (Some(cid1), Some(cid2)) = (&call.call_id, &o.call_id) {
                cid1 == cid2
            } else if let Some(cline) = o.call_line {
                cline == call.source_line
            } else {
                // Adjacent fallback
                o.record_idx > call.record_idx && o.record_idx <= call.record_idx + 2
            }
        });

        if let Some(out) = matching_out {
            matched_outputs.insert(out.record_idx);
            paired.push(PairedTool {
                call_idx: call.record_idx,
                output_idx: out.record_idx,
                output_line: out.source_line,
                action: call.action.clone(),
                is_error: out.is_error,
                content_preview: out.content_preview.clone(),
                original_chars: out.original_chars,
            });
        }
    }

    paired
}

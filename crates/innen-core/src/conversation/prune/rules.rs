//! Phase 3: evaluate the pruning rules against the paired calls.
//!
//! Three rules, in order: a file read superseded by a later write to the same
//! path; a file read made redundant by a later read with no write in between;
//! and a search that returned nothing. Failing outputs and the newest
//! `keep_threshold` records are never pruned, so recent work and error
//! evidence survive untouched.

use std::collections::BTreeMap;

use super::classify::{is_empty_search_output, ToolAction};
use super::pair::PairedTool;

/// A prune decision: `(record_idx, reason_string, chars_saved)`.
pub(super) type PruneDecisions = BTreeMap<usize, (String, usize)>;

pub(super) struct RuleOutcome {
    pub decisions: PruneDecisions,
    pub superseded_reads: usize,
    pub duplicate_reads: usize,
    pub empty_searches: usize,
}

pub(super) fn evaluate_prune_rules(
    paired: &[PairedTool],
    file_writes: &BTreeMap<String, Vec<(usize, usize)>>,
    keep_threshold: usize,
) -> RuleOutcome {
    let mut decisions = PruneDecisions::new();
    let mut superseded_reads = 0;
    let mut duplicate_reads = 0;
    let mut empty_searches = 0;

    // Group file reads by path to detect supersession and redundancy
    let mut path_reads: BTreeMap<String, Vec<&PairedTool>> = BTreeMap::new();
    for tool in paired {
        if let ToolAction::ReadFile { ref path } = tool.action {
            path_reads.entry(path.clone()).or_default().push(tool);
        }
    }

    // Rule 1 & Rule 2: File Reads
    for (path, reads) in path_reads {
        let writes = file_writes.get(&path);

        for (i, read_tool) in reads.iter().enumerate() {
            // Never prune errors or recent records
            if read_tool.is_error || read_tool.output_idx >= keep_threshold {
                continue;
            }

            // Check if superseded by later write
            if let Some(w_list) = writes {
                if let Some((_, write_line)) = w_list
                    .iter()
                    .find(|(w_idx, _)| *w_idx > read_tool.output_idx)
                {
                    let reason =
                        format!("read of '{path}' superseded by write at line {write_line}");
                    decisions.insert(read_tool.output_idx, (reason, read_tool.original_chars));
                    superseded_reads += 1;
                    continue;
                }
            }

            // Check if redundant with a later read of same file (with no writes in between)
            if i + 1 < reads.len() {
                let next_read = reads[i + 1];
                let no_intervening_writes = writes
                    .map(|wl| {
                        !wl.iter().any(|(w_idx, _)| {
                            *w_idx > read_tool.output_idx && *w_idx < next_read.call_idx
                        })
                    })
                    .unwrap_or(true);
                if no_intervening_writes {
                    let reason = format!(
                        "read of '{path}' redundant; re-read at line {}",
                        next_read.output_line
                    );
                    decisions.insert(read_tool.output_idx, (reason, read_tool.original_chars));
                    duplicate_reads += 1;
                    continue;
                }
            }
        }
    }

    // Rule 3: Empty Searches
    for tool in paired {
        if tool.is_error || tool.output_idx >= keep_threshold {
            continue;
        }
        if let ToolAction::Search { ref query } = tool.action {
            if is_empty_search_output(&tool.content_preview) {
                let reason = format!("search for '{query}' returned 0 matches");
                decisions.insert(tool.output_idx, (reason, tool.original_chars));
                empty_searches += 1;
            }
        }
    }

    RuleOutcome {
        decisions,
        superseded_reads,
        duplicate_reads,
        empty_searches,
    }
}

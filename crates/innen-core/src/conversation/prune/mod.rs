//! Deterministic rule-based tool pruning.
//!
//! Drops or truncates provably stale tool outputs (e.g. file reads superseded by
//! subsequent writes/edits, duplicate identical reads, empty search results)
//! without invoking an external LLM.
//!
//! Preserves user instructions, assistant responses, failing tool outputs,
//! recent turns, and exact line/SHA-256 provenance.
//!
//! One phase per file: classify a call, scan the page, pair calls with outputs,
//! evaluate the rules, apply the rewrite.

mod apply;
mod classify;
mod page;
mod pair;
mod rules;
mod scan;

pub use page::prune_page;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct PruneOptions {
    /// Number of newest records to unconditionally keep untouched. Default: 4.
    pub preserve_recent: usize,
    /// Truncate preview head characters for retained notes. Default: 120.
    pub truncate_head_chars: usize,
}

impl Default for PruneOptions {
    fn default() -> Self {
        Self {
            preserve_recent: 4,
            truncate_head_chars: 120,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PruneReceipt {
    pub total_records: usize,
    pub pruned_count: usize,
    pub superseded_reads: usize,
    pub duplicate_reads: usize,
    pub empty_searches: usize,
    pub saved_chars_estimate: usize,
}

#[cfg(test)]
mod tests;

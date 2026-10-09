//! `prune_page`: run the four prune phases over one page and build the receipt.
//!
//! The phases are separate files because they answer different questions: what
//! did this page call, which output belonged to which call, what is stale, and
//! how is the page rewritten. This file owns only their order and the receipt
//! the caller reads.

use super::super::Page;
use super::apply::apply_prune_decisions;
use super::pair::pair_calls_with_outputs;
use super::rules::evaluate_prune_rules;
use super::scan::scan_records;
use super::PruneOptions;
use super::PruneReceipt;

/// Prune stale tool results in a `Page` deterministically according to rules.
pub fn prune_page(page: &mut Page, options: &PruneOptions) -> PruneReceipt {
    let mut receipt = PruneReceipt {
        total_records: page.records.len(),
        ..Default::default()
    };

    if page.records.is_empty() {
        return receipt;
    }

    let keep_threshold = page.records.len().saturating_sub(options.preserve_recent);

    let scan = scan_records(&page.records);
    let paired = pair_calls_with_outputs(&scan.calls, &scan.outputs);
    let outcome = evaluate_prune_rules(&paired, &scan.file_writes, keep_threshold);

    receipt.superseded_reads = outcome.superseded_reads;
    receipt.duplicate_reads = outcome.duplicate_reads;
    receipt.empty_searches = outcome.empty_searches;

    let applied = apply_prune_decisions(page, &outcome.decisions);
    receipt.pruned_count = applied.pruned_count;
    receipt.saved_chars_estimate = applied.saved_chars_estimate;

    if receipt.pruned_count > 0 {
        page.warnings.push(format!(
            "pruned {} stale tool output(s) using deterministic rules (saved ~{} chars)",
            receipt.pruned_count, receipt.saved_chars_estimate
        ));
    }

    receipt
}

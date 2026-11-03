//! Per-term retry for a request that matched no node as a whole.

use std::collections::HashMap;

use serde_json::Value;

use super::node_identity::node_identity_text;

/// Per-term fallback keeps only the best-covered candidates: one generic term
/// can match dozens of nodes, and a long tail is noise, not context.
pub(super) const FALLBACK_CANDIDATE_LIMIT: usize = 10;

/// Candidates ranked by term coverage, the terms no node contained, and
/// whether the list was cut at `FALLBACK_CANDIDATE_LIMIT`.
pub(super) struct TermFallback {
    pub candidates: Vec<(String, f64, String)>,
    pub unmatched: Vec<String>,
    pub truncated: bool,
}

/// Rank nodes by how many identity terms their identity fields contain.
pub(super) fn per_term_candidates(
    nodes: &HashMap<String, Value>,
    identity_terms: &[&str],
) -> TermFallback {
    let lowered: Vec<String> = identity_terms.iter().map(|t| t.to_lowercase()).collect();
    let mut covered = vec![false; lowered.len()];
    let mut candidates: Vec<(String, f64, String)> = Vec::new();
    for (id, node) in nodes {
        let hay = node_identity_text(node).to_lowercase();
        let hits: Vec<usize> = (0..lowered.len())
            .filter(|i| hay.contains(&lowered[*i]))
            .collect();
        for i in &hits {
            covered[*i] = true;
        }
        if !hits.is_empty() {
            candidates.push((
                id.clone(),
                hits.len() as f64 / lowered.len() as f64,
                format!("term_coverage:{}/{}", hits.len(), lowered.len()),
            ));
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let truncated = candidates.len() > FALLBACK_CANDIDATE_LIMIT;
    candidates.truncate(FALLBACK_CANDIDATE_LIMIT);
    let unmatched = if candidates.is_empty() {
        Vec::new()
    } else {
        identity_terms
            .iter()
            .zip(covered)
            .filter(|(_, hit)| !hit)
            .map(|(t, _)| (*t).to_string())
            .collect()
    };
    TermFallback {
        candidates,
        unmatched,
        truncated,
    }
}

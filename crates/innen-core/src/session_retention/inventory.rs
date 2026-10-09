//! Store-wide inventory: enumerate every native session once, assess each one,
//! and aggregate the result into a summary safe to print.

use super::*;

pub fn inventory(
    root: &Path,
    source: Option<Source>,
    source_root: Option<&Path>,
    retention_days: u64,
) -> Result<Vec<Assessment>, Error> {
    let candidates = find_all_candidates(source, source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?;
    inventory_of(root, &candidates, retention_days)
}

/// Assess an already enumerated store. Enumerating reads the tail of every
/// native session, so callers that touch many sessions enumerate once and
/// reuse the list.
pub(super) fn inventory_of(
    root: &Path,
    candidates: &[Candidate],
    retention_days: u64,
) -> Result<Vec<Assessment>, Error> {
    let graph = load_graph(root)?;
    candidates
        .iter()
        .map(|candidate| {
            Ok(
                assess_candidate_with_graph(&graph, candidate, retention_days).unwrap_or_else(
                    |error| Assessment {
                        session_id: candidate.id.clone(),
                        source: candidate.source.as_str().into(),
                        modified: candidate.modified.clone(),
                        cutoff_utc: cutoff_utc(retention_days),
                        source_bytes: 0,
                        source_sha256: String::new(),
                        eligible: false,
                        blockers: vec![format!("assessment failed: {error}")],
                        blocker_codes: vec![BLOCKER_ASSESSMENT_ERROR.into()],
                        targets: candidate_targets(candidate),
                    },
                ),
            )
        })
        .collect()
}

pub fn summarize_inventory(assessments: &[Assessment]) -> InventorySummary {
    let mut summary = InventorySummary {
        schema: "innen.session-retention-summary.v1",
        total: assessments.len(),
        eligible: 0,
        eligible_bytes: 0,
        by_source: BTreeMap::new(),
        blocker_codes: BTreeMap::new(),
    };
    for assessment in assessments {
        let source = summary
            .by_source
            .entry(assessment.source.clone())
            .or_default();
        source.total += 1;
        if assessment.eligible {
            source.eligible += 1;
            summary.eligible += 1;
            summary.eligible_bytes = summary
                .eligible_bytes
                .saturating_add(assessment.source_bytes);
        }
        for code in &assessment.blocker_codes {
            *summary.blocker_codes.entry(code.clone()).or_default() += 1;
        }
    }
    summary
}

use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct SweepItem {
    pub session_id: String,
    pub source: String,
    pub status: String,
    pub source_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_bytes: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepReceipt {
    pub schema: &'static str,
    pub total: usize,
    pub eligible: usize,
    /// Closed sessions whose only blocker is a missing extraction; sweep compacts them first.
    pub compactable: usize,
    pub blocked: usize,
    pub compacted: usize,
    pub removed: usize,
    pub failed: usize,
    pub native_bytes_removed: u64,
    pub compact_bytes_written: u64,
    pub executed: bool,
    pub items: Vec<SweepItem>,
}

fn only_missing_extraction(assessment: &Assessment) -> bool {
    assessment.blocker_codes == [BLOCKER_ATTESTATION_MISSING]
}

pub fn sweep(
    root: &Path,
    source: Option<Source>,
    source_root: Option<&Path>,
    retention_days: u64,
    execute: bool,
) -> Result<SweepReceipt, Error> {
    let assessments = inventory(root, source, source_root, retention_days)?;
    let eligible = assessments.iter().filter(|item| item.eligible).count();
    let compactable = assessments
        .iter()
        .filter(|item| only_missing_extraction(item))
        .count();
    let mut receipt = SweepReceipt {
        schema: "innen.session-retention-sweep.v2",
        total: assessments.len(),
        eligible,
        compactable,
        blocked: assessments.len() - eligible - compactable,
        compacted: 0,
        removed: 0,
        failed: 0,
        native_bytes_removed: 0,
        compact_bytes_written: 0,
        executed: execute,
        items: Vec::new(),
    };
    // Children first: deleting a Codex parent also deletes its subagent sessions,
    // so each child must be compacted and purged before its parent is reached.
    let parents: BTreeMap<String, String> = find_all_candidates(source, source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?
        .into_iter()
        .filter_map(|candidate| candidate.parent_id.map(|parent| (candidate.id, parent)))
        .collect();
    let depth = |id: &str| {
        let mut depth = 0usize;
        let mut current = id;
        while let Some(parent) = parents.get(current) {
            depth += 1;
            if depth > 64 {
                break; // malformed cycle; order is best effort, purge still guards
            }
            current = parent;
        }
        depth
    };
    let mut work: Vec<Assessment> = assessments
        .into_iter()
        .filter(|item| item.eligible || only_missing_extraction(item))
        .collect();
    work.sort_by_key(|item| std::cmp::Reverse(depth(&item.session_id)));
    for assessment in work {
        let needs_compact = !assessment.eligible;
        if !execute {
            receipt.items.push(SweepItem {
                session_id: assessment.session_id,
                source: assessment.source,
                status: if needs_compact {
                    "compactable"
                } else {
                    "eligible"
                }
                .into(),
                source_bytes: assessment.source_bytes,
                compact_bytes: None,
                detail: None,
            });
            continue;
        }
        let mut compact_bytes = None;
        let mut native_bytes = assessment.source_bytes;
        let outcome = (|| -> Result<(), Error> {
            if needs_compact {
                let source = Source::parse(&assessment.source)
                    .map_err(|error| Error::Proof(error.to_string()))?;
                let candidate = find_candidate(source, &assessment.session_id, source_root)?;
                let record = compact_and_attest(root, &candidate)?;
                compact_bytes = Some(record.bytes);
                native_bytes = record.source_bytes;
            }
            purge(
                root,
                &assessment.source,
                &assessment.session_id,
                source_root,
                retention_days,
                true,
            )
            .map(|_| ())
        })();
        if let Some(bytes) = compact_bytes {
            receipt.compacted += 1;
            receipt.compact_bytes_written = receipt.compact_bytes_written.saturating_add(bytes);
        }
        let (status, detail) = match outcome {
            Ok(()) => {
                receipt.removed += 1;
                receipt.native_bytes_removed =
                    receipt.native_bytes_removed.saturating_add(native_bytes);
                ("removed", None)
            }
            Err(error) => {
                receipt.failed += 1;
                ("failed", Some(error.to_string()))
            }
        };
        receipt.items.push(SweepItem {
            session_id: assessment.session_id,
            source: assessment.source,
            status: status.into(),
            source_bytes: native_bytes,
            compact_bytes,
            detail,
        });
    }
    Ok(receipt)
}

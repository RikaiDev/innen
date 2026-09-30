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
    /// Swept sessions whose native files have other hard links: their bytes
    /// stay on disk until the other links are removed too.
    pub hard_linked: usize,
    pub hard_linked_bytes: u64,
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
    // One enumeration for the whole sweep: it reads the tail of every session.
    let store = find_all_candidates(source, source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?;
    let assessments = inventory_of(root, &store, retention_days)?;
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
        hard_linked: 0,
        hard_linked_bytes: 0,
        executed: execute,
        items: Vec::new(),
    };
    // Children first: deleting a Codex parent also deletes its subagent sessions,
    // so each child must be compacted and purged before its parent is reached.
    let parents: BTreeMap<&str, &str> = store
        .iter()
        .filter_map(|candidate| Some((candidate.id.as_str(), candidate.parent_id.as_deref()?)))
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
        // Measured before anything is deleted; reported in dry runs too.
        let (other_links, linked_bytes) = hard_links(&assessment.targets);
        let link_note = (other_links > 0).then(|| {
            format!(
                "{other_links} other hard link(s) keep {linked_bytes} bytes on disk; removing this path frees no space"
            )
        });
        if other_links > 0 {
            receipt.hard_linked += 1;
            receipt.hard_linked_bytes = receipt.hard_linked_bytes.saturating_add(linked_bytes);
        }
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
                detail: link_note,
            });
            continue;
        }
        let mut compact_bytes = None;
        let mut native_bytes = assessment.source_bytes;
        let outcome = (|| -> Result<(), Error> {
            let candidate = store
                .iter()
                .find(|candidate| {
                    candidate.id == assessment.session_id
                        && candidate.source.as_str() == assessment.source
                })
                .ok_or_else(|| {
                    Error::Proof(format!(
                        "session not found: {}:{}",
                        assessment.source, assessment.session_id
                    ))
                })?;
            if needs_compact {
                let record = compact_and_attest(root, candidate)?;
                compact_bytes = Some(record.bytes);
                native_bytes = record.source_bytes;
            }
            purge_candidate(root, candidate, &store, source_root, retention_days, true).map(|_| ())
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
                ("removed", link_note)
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

//! Deleting an attested native bundle: re-assess, guard children, hand the
//! deletion to the source's own adapter, then journal what actually happened.

use super::*;

pub fn purge(
    root: &Path,
    source_name: &str,
    session_id: &str,
    source_root: Option<&Path>,
    retention_days: u64,
    execute: bool,
) -> Result<PurgeReceipt, Error> {
    let source = Source::parse(source_name).map_err(|error| Error::Proof(error.to_string()))?;
    let id = sources::validate_id(session_id).map_err(|error| Error::Proof(error.to_string()))?;
    let candidates = find_all_candidates(Some(source), source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?;
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| Error::Proof(format!("session not found: {}:{id}", source.as_str())))?;
    purge_candidate(
        root,
        candidate,
        &candidates,
        source_root,
        retention_days,
        execute,
    )
}

/// Purge one session of an already enumerated store (see [`inventory_of`]).
pub(super) fn purge_candidate(
    root: &Path,
    candidate: &Candidate,
    store: &[Candidate],
    source_root: Option<&Path>,
    retention_days: u64,
    execute: bool,
) -> Result<PurgeReceipt, Error> {
    let source = candidate.source;
    let session_id = candidate.id.as_str();
    // `codex delete` also removes a thread's subagent sessions: purge children first.
    if candidate.source == Source::Codex {
        let children = live_children(candidate, store);
        if !children.is_empty() {
            return Err(Error::Proof(format!(
                "codex delete would also remove child sessions; purge them first: {}",
                children.join(",")
            )));
        }
    }
    let attempt_id = if execute {
        Some(record_purge_attempt(root, candidate, retention_days)?)
    } else {
        None
    };
    let assessment = match assess_candidate(root, candidate, retention_days) {
        Ok(assessment) => assessment,
        Err(error) => {
            if let Some(id) = attempt_id.as_deref() {
                record_purge_failure(root, id, candidate, &error.to_string())?;
            }
            return Err(error);
        }
    };
    if !assessment.eligible {
        let error = Error::Proof(assessment.blockers.join("; "));
        if let Some(id) = attempt_id.as_deref() {
            record_purge_failure(root, id, candidate, &error.to_string())?;
        }
        return Err(error);
    }
    if !execute {
        let (other_hard_links, hard_linked_bytes) = hard_links(&assessment.targets);
        return Ok(PurgeReceipt {
            session_id: session_id.into(),
            source: source.as_str().into(),
            executed: false,
            deleted_targets: vec![],
            source_sha256: assessment.source_sha256,
            other_hard_links,
            hard_linked_bytes,
        });
    }
    let (other_hard_links, hard_linked_bytes) = hard_links(&assessment.targets);

    let deleted_targets = match delete_candidate(candidate, &assessment, source_root) {
        Ok(targets) => targets,
        Err(error) => {
            if let Some(id) = attempt_id.as_deref() {
                record_purge_failure(root, id, candidate, &error.to_string())?;
            }
            return Err(error);
        }
    };
    let receipt = json!({
        "id": format!("session-purge:{}:{}:{}", source.as_str(), session_id, &assessment.source_sha256[..16]),
        "type": "SessionPurgeReceipt",
        "label": format!("purged {}:{}", source.as_str(), session_id),
        "provenance": "innen retention purge --execute",
        "schema": SCHEMA,
        "session_id": session_id,
        "source": source.as_str(),
        "source_sha256": assessment.source_sha256,
        "attempt_id": attempt_id,
        "deleted_targets": deleted_targets,
        "observed_utc": observed_utc_now(),
    });
    Journal::open(root)
        .map_err(|error| Error::Journal(error.to_string()))?
        .append("node.upsert", &receipt)
        .map_err(|error| Error::Journal(error.to_string()))?;
    Ok(PurgeReceipt {
        session_id: session_id.into(),
        source: source.as_str().into(),
        executed: true,
        deleted_targets: serde_json::from_value(receipt["deleted_targets"].clone())
            .unwrap_or_default(),
        source_sha256: assessment.source_sha256,
        other_hard_links,
        hard_linked_bytes,
    })
}

/// Children of `candidate` that still exist on disk; `store` may predate deletions.
fn live_children(candidate: &Candidate, store: &[Candidate]) -> Vec<String> {
    store
        .iter()
        .filter(|child| {
            child.parent_id.as_deref() == Some(candidate.id.as_str()) && child.path.exists()
        })
        .map(|child| child.id.clone())
        .collect()
}

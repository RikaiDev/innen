use super::*;
pub(super) fn record_purge_attempt(
    root: &Path,
    candidate: &Candidate,
    retention_days: u64,
) -> Result<String, Error> {
    let observed_utc = observed_utc_now();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = format!(
        "session-purge-attempt:{}:{}:{}",
        candidate.source.as_str(),
        candidate.id,
        nonce
    );
    append_retention_node(
        root,
        &json!({
            "id": id,
            "type": "SessionPurgeAttempt",
            "label": format!("purge attempt {}:{}", candidate.source.as_str(), candidate.id),
            "provenance": "innen retention purge --execute",
            "schema": SCHEMA,
            "session_id": candidate.id,
            "source": candidate.source.as_str(),
            "retention_days": retention_days,
            "status": "started",
            "observed_utc": observed_utc,
        }),
    )?;
    Ok(id)
}

pub(super) fn record_purge_failure(
    root: &Path,
    attempt_id: &str,
    candidate: &Candidate,
    reason: &str,
) -> Result<(), Error> {
    append_retention_node(
        root,
        &json!({
            "id": format!("{attempt_id}:failure"),
            "type": "SessionPurgeFailure",
            "label": format!("purge failed {}:{}", candidate.source.as_str(), candidate.id),
            "provenance": "innen retention purge --execute",
            "schema": SCHEMA,
            "attempt_id": attempt_id,
            "session_id": candidate.id,
            "source": candidate.source.as_str(),
            "status": "failed",
            "reason": reason,
            "observed_utc": observed_utc_now(),
        }),
    )
}

fn append_retention_node(root: &Path, payload: &Value) -> Result<(), Error> {
    Journal::open(root)
        .map_err(|error| Error::Journal(error.to_string()))?
        .append("node.upsert", payload)
        .map(|_| ())
        .map_err(|error| Error::Journal(error.to_string()))
}

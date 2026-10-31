//! Deleting an Antigravity session from the store that owns it.

use super::super::*;
use super::antigravity_store_root;
use super::sqlite::{read_shared_store, run_sqlite_mutation};

pub(super) fn delete_antigravity(
    candidate: &Candidate,
    assessment: &Assessment,
) -> Result<(), Error> {
    let store_root = antigravity_store_root(&assessment.targets)
        .ok_or_else(|| Error::Purge("Antigravity store root is not canonical".into()))?;
    let summary_db = store_root.join("conversation_summaries.db");
    if summary_db.exists() {
        let state_sql = format!(
            "SELECT json_object('not_fully_idle',not_fully_idle,'killed',killed) FROM conversation_summaries WHERE conversation_id='{}'",
            candidate.id
        );
        let rows = read_shared_store(&summary_db, &state_sql)?;
        if rows.iter().any(|row| {
            row.get("not_fully_idle")
                .and_then(Value::as_i64)
                .is_some_and(|value| value != 0)
        }) {
            return Err(Error::Purge(
                "Antigravity marks the session as not fully idle".into(),
            ));
        }
        let delete_sql = format!(
            "PRAGMA busy_timeout=5000; BEGIN IMMEDIATE; DELETE FROM conversation_summaries WHERE conversation_id='{}'; COMMIT;",
            candidate.id
        );
        run_sqlite_mutation(&summary_db, &delete_sql)?;
    }
    for target in &assessment.targets {
        let metadata = fs::symlink_metadata(target).map_err(|error| io(target, error))?;
        if metadata.is_dir() {
            fs::remove_dir_all(target).map_err(|error| io(target, error))?;
        } else {
            fs::remove_file(target).map_err(|error| io(target, error))?;
        }
    }
    if assessment.targets.iter().any(|target| target.exists()) {
        return Err(Error::Purge(
            "Antigravity native targets still exist after deletion".into(),
        ));
    }
    Ok(())
}

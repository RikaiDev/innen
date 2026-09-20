use super::activity::{
    session_activity, snapshot_target_identities, verify_target_identities, Activity,
};
use super::*;
pub(super) fn delete_candidate(
    candidate: &Candidate,
    assessment: &Assessment,
) -> Result<Vec<PathBuf>, Error> {
    let identities = snapshot_target_identities(&assessment.targets)?;
    let current = bundle(candidate)?;
    if current.sha256 != assessment.source_sha256 || current.bytes != assessment.source_bytes {
        return Err(Error::Purge(
            "source changed after eligibility check".into(),
        ));
    }
    verify_target_identities(&identities)?;
    match session_activity(&assessment.targets) {
        Activity::Idle => {}
        Activity::Active => {
            return Err(Error::Purge(
                "native session became active after eligibility check".into(),
            ))
        }
        Activity::Unknown(detail) => {
            return Err(Error::Purge(format!(
                "native session activity could not be verified: {detail}"
            )))
        }
    }
    verify_target_identities(&identities)?;

    match candidate.source {
        Source::Codex => run_delete("codex", &["delete", "--force", &candidate.id])?,
        Source::Opencode => run_delete("opencode", &["session", "delete", &candidate.id])?,
        Source::Claude | Source::Qwen => {
            fs::remove_file(&candidate.path).map_err(|error| io(&candidate.path, error))?;
        }
        Source::Antigravity => delete_antigravity(candidate, assessment)?,
        _ => {
            return Err(Error::Purge(
                "source has no supported delete adapter".into(),
            ))
        }
    }
    Ok(assessment.targets.clone())
}

fn delete_antigravity(candidate: &Candidate, assessment: &Assessment) -> Result<(), Error> {
    let store_root = assessment
        .targets
        .iter()
        .find_map(|target| {
            target.ancestors().find(|ancestor| {
                ancestor
                    .file_name()
                    .is_some_and(|name| name == "antigravity-cli")
            })
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::Purge("Antigravity store root is not canonical".into()))?;
    let summary_db = store_root.join("conversation_summaries.db");
    if summary_db.exists() {
        let state_sql = format!(
            "SELECT json_object('not_fully_idle',not_fully_idle,'killed',killed) FROM conversation_summaries WHERE conversation_id='{}'",
            candidate.id
        );
        let rows = sources::sqlite(&summary_db, &state_sql)
            .map_err(|error| Error::Purge(error.to_string()))?;
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

pub(super) fn run_sqlite_mutation(path: &Path, sql: &str) -> Result<(), Error> {
    let output = Command::new("sqlite3")
        .arg(path)
        .arg(sql)
        .output()
        .map_err(|error| Error::Purge(format!("sqlite3: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr)
        .chars()
        .take(512)
        .collect::<String>();
    Err(Error::Purge(format!(
        "sqlite3 mutation failed ({}): {}",
        output.status,
        detail.trim()
    )))
}

pub(super) fn run_delete(program: &str, args: &[&str]) -> Result<(), Error> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| Error::Purge(format!("{program}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(512)
            .collect::<String>();
        let detail = detail.trim();
        if detail.is_empty() {
            Err(Error::Purge(format!("{program} exited {}", output.status)))
        } else {
            Err(Error::Purge(format!(
                "{program} exited {}: {detail}",
                output.status
            )))
        }
    }
}

use super::activity::{
    liveness_targets, session_activity, snapshot_target_identities, verify_target_identities,
    Activity,
};
use super::*;

/// Sources this module can delete.
///
/// The command line derives its `--source` values from this list, so a source
/// with a working adapter is always reachable from the command line.
pub(super) const DELETE_ADAPTER_SOURCES: &[Source] = &[
    Source::Antigravity,
    Source::Codex,
    Source::Claude,
    Source::Copilot,
    Source::Grok,
    Source::Mcode,
    Source::Opencode,
    Source::Qwen,
];

/// Sources this module refuses to delete because no adapter exists for them.
///
/// Every `Source` variant is either listed here or in `DELETE_ADAPTER_SOURCES`;
/// `every_source_variant_is_classified` keeps that exhaustive.
pub(super) const NO_DELETE_ADAPTER_SOURCES: &[Source] = &[Source::Cursor, Source::Gemini, Source::Vscode];

/// Every source name the retention pipeline accepts, aliases included.
///
/// The names come from the sources that own a delete adapter, so the command
/// line never offers a source the pipeline would refuse to reclaim.
pub fn retention_source_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = DELETE_ADAPTER_SOURCES
        .iter()
        .map(|source| source.as_str())
        .collect();
    names.push("agy");
    names.push("minimax");
    names
}

pub(super) fn delete_candidate(
    candidate: &Candidate,
    assessment: &Assessment,
    source_root: Option<&Path>,
) -> Result<Vec<PathBuf>, Error> {
    // Codex and OpenCode delete through their own CLI, which only knows its
    // canonical store: with a --source-root override it would delete the
    // same id from the real store while the assessed copy stays in place.
    if source_root.is_some() && matches!(candidate.source, Source::Codex | Source::Opencode) {
        return Err(Error::Purge(format!(
            "{} deletes through its own CLI and its canonical store; refusing with --source-root",
            candidate.source.as_str()
        )));
    }
    let identities = snapshot_target_identities(&assessment.targets)?;
    let current = bundle(candidate)?;
    if current.sha256 != assessment.source_sha256 || current.bytes != assessment.source_bytes {
        return Err(Error::Purge(
            "source changed after eligibility check".into(),
        ));
    }
    verify_target_identities(&identities)?;
    match session_activity(&liveness_targets(candidate.source, &assessment.targets)) {
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
        // --standalone keeps the delete off the shared background service: that
        // service holds the session database open for the whole host, so a
        // delete that goes through it leaves the store locked and every
        // later session reads as active.
        Source::Opencode => {
            let args = opencode_delete_args(&candidate.id);
            let borrowed = args.iter().map(String::as_str).collect::<Vec<&str>>();
            run_delete("opencode", &borrowed)?
        }
        Source::Claude | Source::Qwen => {
            fs::remove_file(&candidate.path).map_err(|error| io(&candidate.path, error))?;
        }
        Source::Antigravity => delete_antigravity(candidate, assessment)?,
        Source::Mcode | Source::Copilot | Source::Grok => {
            delete_indexed_session(candidate, assessment)?
        }
        source @ (Source::Cursor | Source::Gemini | Source::Vscode) => {
            debug_assert!(
                NO_DELETE_ADAPTER_SOURCES.contains(&source),
                "a source without a delete adapter must stay classified"
            );
            return Err(Error::Purge(format!(
                "source has no supported delete adapter: {}",
                source.as_str()
            )))
        }
    }
    // A receipt may only claim what actually happened on disk.
    let remaining: Vec<_> = if candidate.source == Source::Opencode {
        Vec::new() // the database file is shared; the CLI removes the rows
    } else {
        assessment
            .targets
            .iter()
            .filter(|target| target.exists())
            .collect()
    };
    if !remaining.is_empty() {
        return Err(Error::Purge(format!(
            "native targets still exist after deletion: {}",
            remaining
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
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

/// Run a mutation against a SQLite database that another process may own.
///
/// A live owner is not a reason to stop: the shared store is supposed to be
/// reclaimable while its tool is running. SQLite refuses to open a WAL
/// database for writing when the WAL-index files are missing while another
/// process still holds the database open (`SQLITE_CANTOPEN`), and it refuses
/// to write while another process holds the write lock (`SQLITE_BUSY`). Both
/// are transient: rebuilding the WAL-index and waiting for the lock clears
/// them, so this retries after repairing the index.
pub(super) fn run_sqlite_mutation(path: &Path, sql: &str) -> Result<(), Error> {
    let mut last = String::new();
    for attempt in 0..SQLITE_MUTATION_ATTEMPTS {
        let output = Command::new("sqlite3")
            .arg(path)
            .arg(sql)
            .output()
            .map_err(|error| Error::Purge(format!("sqlite3: {error}")))?;
        if output.status.success() {
            return Ok(());
        }
        last = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(512)
            .collect::<String>();
        last = last.trim().to_string();
        // Rebuild the WAL-index so the next attempt can open the database.
        let _ = Command::new("sqlite3")
            .arg(path)
            .arg("PRAGMA wal_checkpoint(TRUNCATE);")
            .output();
        if attempt + 1 < SQLITE_MUTATION_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(
                SQLITE_MUTATION_BACKOFF_MS * (attempt as u64 + 1),
            ));
        }
    }
    Err(Error::Purge(format!(
        "sqlite3 mutation failed after {} attempts: {}",
        SQLITE_MUTATION_ATTEMPTS, last
    )))
}

/// Attempts and backoff for a shared-store mutation. A live owner is the
/// normal case, so the retry budget has to outlive a normal lock wait.
const SQLITE_MUTATION_ATTEMPTS: usize = 6;
const SQLITE_MUTATION_BACKOFF_MS: u64 = 500;


/// Stores that keep one directory per session next to a shared index
/// database.
///
/// The directory is the session; the index row is what makes it visible.
/// Removing the directory alone would leave the index pointing at a session
/// that no longer exists, so both go together. The index database is shared
/// with the tool, so its mutation goes through the retrying helper that
/// tolerates a live owner.
fn delete_indexed_session(
    candidate: &Candidate,
    assessment: &Assessment,
) -> Result<(), Error> {
    let layout = indexed_store_layout(candidate)?;
    for statement in layout.statements {
        run_sqlite_mutation(&layout.index_db, &statement)?;
    }
    let directory = session_directory(candidate);
    if directory.is_dir() {
        fs::remove_dir_all(&directory).map_err(|error| io(&directory, error))?;
    } else if directory.exists() {
        fs::remove_file(&directory).map_err(|error| io(&directory, error))?;
    }
    if assessment.targets.iter().any(|target| target.exists()) {
        return Err(Error::Purge(
            "indexed session targets still exist after deletion".into(),
        ));
    }
    Ok(())
}

struct IndexedStoreLayout {
    index_db: PathBuf,
    statements: Vec<String>,
}

/// The index database and the statements that remove one session from it.
///
/// The session id comes from the command line and is validated to be a UUID
/// or a native id of `ses_`/`mvs_` plus alphanumerics and underscores, so it
/// cannot carry a quote and is safe to inline.
fn indexed_store_layout(candidate: &Candidate) -> Result<IndexedStoreLayout, Error> {
    let id = &candidate.id;
    let (relative_db, statements): (&str, Vec<(&str, String)>) = match candidate.source {
        Source::Mcode => (
            "sqlite/runtime-state.sqlite",
            vec![
                (
                    "local_runtime_message_rows",
                    format!("DELETE FROM local_runtime_message_rows WHERE session_id='{id}';"),
                ),
                (
                    "local_runtime_messages",
                    format!("DELETE FROM local_runtime_messages WHERE session_id='{id}';"),
                ),
                (
                    "local_runtime_session_assets",
                    format!("DELETE FROM local_runtime_session_assets WHERE session_id='{id}';"),
                ),
                (
                    "local_runtime_session_locks",
                    format!("DELETE FROM local_runtime_session_locks WHERE session_id='{id}';"),
                ),
                (
                    "local_runtime_session_projection_watermarks",
                    format!(
                        "DELETE FROM local_runtime_session_projection_watermarks WHERE session_id='{id}';"
                    ),
                ),
                (
                    "local_runtime_sessions",
                    format!("DELETE FROM local_runtime_sessions WHERE session_id='{id}';"),
                ),
            ],
        ),
        Source::Copilot => (
            "session-store.db",
            vec![
                (
                    "search_index",
                    format!("DELETE FROM search_index WHERE session_id='{id}';"),
                ),
                (
                    "session_refs",
                    format!("DELETE FROM session_refs WHERE session_id='{id}';"),
                ),
                (
                    "session_files",
                    format!("DELETE FROM session_files WHERE session_id='{id}';"),
                ),
                (
                    "checkpoints",
                    format!("DELETE FROM checkpoints WHERE session_id='{id}';"),
                ),
                (
                    "turns",
                    format!("DELETE FROM turns WHERE session_id='{id}';"),
                ),
                (
                    "sessions",
                    format!("DELETE FROM sessions WHERE id='{id}';"),
                ),
            ],
        ),
        Source::Grok => (
            "session_search.sqlite",
            vec![(
                "session_docs",
                format!("DELETE FROM session_docs WHERE session_id='{id}';"),
            )],
        ),
        _ => {
            return Err(Error::Purge(
                "source has no indexed session store layout".into(),
            ))
        }
    };
    let start = candidate
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| candidate.path.clone());
    let index_db = start
        .ancestors()
        .map(|dir| dir.join(relative_db))
        .find(|path| path.exists())
        .ok_or_else(|| {
            Error::Purge(format!(
                "session index database not found for {}: {}",
                candidate.source.as_str(),
                relative_db
            ))
        })?;
    // A store whose schema does not have every table must still be
    // reclaimable, so a statement for an absent table is dropped instead of
    // failing the whole session.
    let present = existing_tables(&index_db);
    let statements = statements
        .into_iter()
        .filter(|(table, _)| present.iter().any(|name| name == table))
        .map(|(_, statement)| statement)
        .collect();
    Ok(IndexedStoreLayout {
        index_db,
        statements,
    })
}

/// The tables an index database actually has.
fn existing_tables(index_db: &Path) -> Vec<String> {
    let output = Command::new("sqlite3")
        .arg(index_db)
        .arg("SELECT name FROM sqlite_master WHERE type='table';")
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// The directory that holds one session's files.
///
/// Mcode, Copilot and Grok each keep a directory per session, and the
/// transcript that innen reads is one file inside it. Removing only that
/// file would leave the rest of the session behind.
fn session_directory(candidate: &Candidate) -> PathBuf {
    candidate
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| candidate.path.clone())
}

/// Arguments for `opencode session delete`.
///
/// The delete runs against a private server: the shared background service
/// keeps the session database open for the whole host, and a delete routed
/// through it leaves that handle behind.
pub(super) fn opencode_delete_args(session_id: &str) -> Vec<String> {
    vec![
        "session".to_string(),
        "delete".to_string(),
        session_id.to_string(),
        "--standalone".to_string(),
    ]
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

//! Reclaiming a session that lives next to a shared index database.

use super::super::*;

/// Read from a SQLite database that another process may own.
///
/// A hot WAL database cannot always be opened read-only: recovering the WAL
/// needs write access to the shared index, and SQLite reports that as
/// `unable to open database file`. The same repair that clears a blocked
/// mutation clears a blocked read, so the read retries after rebuilding the
/// WAL-index instead of failing the whole session.
pub(super) fn read_shared_store(path: &Path, sql: &str) -> Result<Vec<Value>, Error> {
    let mut last = String::new();
    for attempt in 0..SQLITE_MUTATION_ATTEMPTS {
        match sources::sqlite(path, sql) {
            Ok(rows) => return Ok(rows),
            Err(error) => {
                last = error.to_string();
            }
        }
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
        "sqlite3 read failed after {} attempts: {last}",
        SQLITE_MUTATION_ATTEMPTS
    )))
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
pub(in crate::session_retention) fn run_sqlite_mutation(
    path: &Path,
    sql: &str,
) -> Result<(), Error> {
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
pub(super) fn delete_indexed_session(
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

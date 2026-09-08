//! Private, append-only storage for validated task history envelopes.
//!
//! Each task has a SHA-256-addressed directory.  Snapshots are immutable bytes
//! addressed by the validated envelope hash; the task index is append-only and
//! protected by a sibling `fs2` lock.

use super::{source_sha256, validate, Request};
use crate::ids::sha256_hex;
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_STORE_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    History(#[from] super::Error),
    #[error("history store is corrupt: {0}")]
    Corrupt(String),
    #[error("history store I/O: {0}")]
    Io(#[from] io::Error),
    #[error("history save contains an event outside project `{project}` and task `{task_id}`")]
    CrossTaskEvent { project: String, task_id: String },
    #[error("history snapshot `{hash}` already exists with different bytes")]
    SnapshotConflict { hash: String },
    #[error("history save removes prior immutable decision `{id}`")]
    DecisionRemoved { id: String },
    #[error("history save rewrites prior immutable decision `{id}`")]
    DecisionRewritten { id: String },
    #[error("history {kind} exceeds the {max} byte admission bound")]
    TooLarge { kind: &'static str, max: u64 },
}

#[derive(Debug, Serialize)]
pub struct SaveReceipt {
    pub resolution: &'static str,
    pub history_source_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexEntry {
    history_source_sha256: String,
}

fn task_key(project: &str, task_id: &str) -> String {
    sha256_hex(&serde_json::to_vec(&(project, task_id)).expect("task identity serializes"))
}

fn task_dir(store: &Path, project: &str, task_id: &str) -> PathBuf {
    store.join(task_key(project, task_id))
}

fn snapshot_path(task_dir: &Path, hash: &str) -> PathBuf {
    task_dir.join("snapshots").join(format!("{hash}.json"))
}

fn ensure_private_dir(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn open_private_append(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn open_private_lock(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, Error> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    std::io::Read::take(&mut file, MAX_STORE_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STORE_FILE_BYTES {
        return Err(Error::Corrupt(format!(
            "{} exceeds {MAX_STORE_FILE_BYTES} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

fn validate_scope(request: &Request) -> Result<(), Error> {
    validate(request)?;
    if request
        .events
        .iter()
        .any(|event| event.project != request.task.project || event.task_id != request.task.id)
    {
        return Err(Error::CrossTaskEvent {
            project: request.task.project.clone(),
            task_id: request.task.id.clone(),
        });
    }
    Ok(())
}

fn read_snapshot(path: &Path, expected_hash: &str) -> Result<Request, Error> {
    let bytes = read_bounded(path)?;
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|e| Error::Corrupt(format!("{}: {e}", path.display())))?;
    validate_scope(&request).map_err(|e| Error::Corrupt(e.to_string()))?;
    let actual = source_sha256(&request);
    if actual != expected_hash {
        return Err(Error::Corrupt(format!(
            "{} hash is {actual}, expected {expected_hash}",
            path.display()
        )));
    }
    Ok(request)
}

fn read_index(path: &Path) -> Result<Vec<IndexEntry>, Error> {
    let bytes = read_bounded(path)?;
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(Error::Corrupt(format!("{} is incomplete", path.display())));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| Error::Corrupt(format!("{}: {e}", path.display())))?;
    let mut entries = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(Error::Corrupt(format!(
                "{} has blank line {}",
                path.display(),
                line_number + 1
            )));
        }
        let entry: IndexEntry = serde_json::from_str(line).map_err(|e| {
            Error::Corrupt(format!("{} line {}: {e}", path.display(), line_number + 1))
        })?;
        if entry.history_source_sha256.len() != 64
            || !entry
                .history_source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::Corrupt(format!(
                "{} line {} has invalid hash",
                path.display(),
                line_number + 1
            )));
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(Error::Corrupt(format!("{} is empty", path.display())));
    }
    Ok(entries)
}

fn reject_immutable_changes(
    dir: &Path,
    entries: &[IndexEntry],
    request: &Request,
) -> Result<(), Error> {
    let incoming = request
        .events
        .iter()
        .map(|event| {
            (
                event.id.as_str(),
                serde_json::to_vec(event).expect("decision serializes"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut prior = BTreeMap::<String, Vec<u8>>::new();
    for entry in entries {
        let older = read_snapshot(
            &snapshot_path(dir, &entry.history_source_sha256),
            &entry.history_source_sha256,
        )?;
        for event in older.events {
            let bytes = serde_json::to_vec(&event).expect("decision serializes");
            match prior.get(&event.id) {
                Some(previous) if previous != &bytes => {
                    return Err(Error::Corrupt(format!(
                        "prior snapshots rewrite decision `{}`",
                        event.id
                    )));
                }
                Some(_) => {}
                None => {
                    prior.insert(event.id, bytes);
                }
            }
        }
    }
    for (id, old_bytes) in prior {
        let Some(new_bytes) = incoming.get(id.as_str()) else {
            return Err(Error::DecisionRemoved { id });
        };
        if *new_bytes != old_bytes {
            return Err(Error::DecisionRewritten { id });
        }
    }
    Ok(())
}

/// Save a full task envelope as immutable bytes and append its address to the
/// task's private index. Existing snapshots are never overwritten.
pub fn save(store: impl AsRef<Path>, request: Request) -> Result<SaveReceipt, Error> {
    validate_scope(&request)?;
    let hash = source_sha256(&request);
    let bytes = serde_json::to_vec(&request).expect("validated history request serializes");
    if bytes.len() as u64 > MAX_STORE_FILE_BYTES {
        return Err(Error::TooLarge {
            kind: "snapshot",
            max: MAX_STORE_FILE_BYTES,
        });
    }
    let dir = task_dir(store.as_ref(), &request.task.project, &request.task.id);
    ensure_private_dir(&dir)?;
    let snapshots = dir.join("snapshots");
    ensure_private_dir(&snapshots)?;
    let lock_path = dir.join("index.lock");
    let lock = open_private_lock(&lock_path)?;
    lock.lock_exclusive()?;

    let result = (|| -> Result<(), Error> {
        let index = dir.join("index.jsonl");
        let prior_entries = if index.exists() {
            read_index(&index)?
        } else {
            Vec::new()
        };
        reject_immutable_changes(&dir, &prior_entries, &request)?;
        let line = serde_json::to_string(&IndexEntry {
            history_source_sha256: hash.clone(),
        })
        .expect("index entry serializes");
        let index_len = match fs::metadata(&index) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(Error::Io(error)),
        };
        if index_len + line.len() as u64 + 1 > MAX_STORE_FILE_BYTES {
            return Err(Error::TooLarge {
                kind: "index",
                max: MAX_STORE_FILE_BYTES,
            });
        }
        let snapshot = snapshot_path(&dir, &hash);
        match read_bounded(&snapshot) {
            Ok(existing) if existing != bytes => {
                return Err(Error::SnapshotConflict { hash: hash.clone() })
            }
            Ok(_) => {}
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                let mut file = create_private_file(&snapshot)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            Err(error) => return Err(error),
        }
        let mut file = open_private_append(&index)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    drop(lock);
    result?;
    Ok(SaveReceipt {
        resolution: "saved",
        history_source_sha256: hash,
    })
}

/// Load and validate the latest immutable full envelope for an exact task.
pub fn load(store: impl AsRef<Path>, project: &str, task_id: &str) -> Result<Request, Error> {
    Ok(load_with_snapshot(store, project, task_id)?.0)
}

/// Load the latest envelope with the immutable snapshot file that supplied it.
pub fn load_with_snapshot(
    store: impl AsRef<Path>,
    project: &str,
    task_id: &str,
) -> Result<(Request, PathBuf), Error> {
    if project.trim().is_empty() || task_id.trim().is_empty() {
        return Err(Error::Corrupt("project and task must be nonempty".into()));
    }
    let dir = task_dir(store.as_ref(), project, task_id);
    let lock = OpenOptions::new().read(true).open(dir.join("index.lock"))?;
    lock.lock_shared()?;
    let result = (|| -> Result<(Request, PathBuf), Error> {
        let entries = read_index(&dir.join("index.jsonl"))?;
        let latest = entries.last().expect("non-empty index checked");
        let snapshot = snapshot_path(&dir, &latest.history_source_sha256);
        let request = read_snapshot(&snapshot, &latest.history_source_sha256)?;
        if request.task.project != project || request.task.id != task_id {
            return Err(Error::Corrupt("index resolves to another task".into()));
        }
        Ok((request, snapshot))
    })();
    drop(lock);
    result
}

//! Deterministic, local-only continuation and checkpoint records.
//! History is strictly append-only: updates append new records without rewriting prior evidence.
//! Private checkpoints remain outside Git via `.innen/.gitignore`.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::resume::system_time_to_rfc3339;
use super::sources::Source;
use super::ReadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStatus {
    Active,
    Blocked,
    Completed,
    Superseded,
}

impl CheckpointStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(s: &str) -> Result<Self, ReadError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "blocked" => Ok(Self::Blocked),
            "completed" => Ok(Self::Completed),
            "superseded" => Ok(Self::Superseded),
            _ => Err(ReadError(format!(
                "unknown checkpoint status: '{s}'; expected active|blocked|completed|superseded"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub session: String,
    pub source: Source,
    pub project: PathBuf,
    pub status: CheckpointStatus,
    pub objective: String,
    #[serde(default)]
    pub completed_work: Vec<String>,
    #[serde(default)]
    pub current_evidence: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    pub next_action: String,
    pub verification: String,
    pub updated_at: String,
}

/// Resolves the project root directory (checking enclosing .git or .innen).
pub fn resolve_project_root(dir: &Path) -> PathBuf {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut curr = Some(canonical.as_path());
    while let Some(p) = curr {
        if p.join(".git").exists() || p.join(".innen").exists() {
            return p.to_path_buf();
        }
        curr = p.parent();
    }
    canonical
}

/// Returns the path to the project's checkpoints.jsonl file under `.innen/`.
pub fn checkpoint_file_path(project_dir: &Path) -> PathBuf {
    let root = resolve_project_root(project_dir);
    root.join(".innen").join("checkpoints.jsonl")
}

/// Ensures `.innen/.gitignore` exists with `*\n` so checkpoints stay outside Git.
pub fn ensure_git_ignored(project_dir: &Path) -> Result<PathBuf, ReadError> {
    let root = resolve_project_root(project_dir);
    let innen_dir = root.join(".innen");
    fs::create_dir_all(&innen_dir).map_err(|e| {
        ReadError(format!(
            "failed to create .innen directory at {}: {e}",
            innen_dir.display()
        ))
    })?;
    let gitignore_path = innen_dir.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, "*\n").map_err(|e| {
            ReadError(format!(
                "failed to write .innen/.gitignore at {}: {e}",
                gitignore_path.display()
            ))
        })?;
    }
    Ok(innen_dir)
}

/// Appends a new checkpoint record to `<project>/.innen/checkpoints.jsonl`.
/// Prior history is never overwritten or deleted.
pub fn record_checkpoint(project_dir: &Path, checkpoint: &Checkpoint) -> Result<(), ReadError> {
    ensure_git_ignored(project_dir)?;
    let path = checkpoint_file_path(project_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| {
            ReadError(format!(
                "failed to open checkpoints file at {}: {e}",
                path.display()
            ))
        })?;

    let line = serde_json::to_string(checkpoint)
        .map_err(|e| ReadError(format!("failed to serialize checkpoint to JSON: {e}")))?;

    writeln!(file, "{line}").map_err(|e| {
        ReadError(format!(
            "failed to write checkpoint to {}: {e}",
            path.display()
        ))
    })?;

    file.flush().map_err(|e| {
        ReadError(format!(
            "failed to flush checkpoints file at {}: {e}",
            path.display()
        ))
    })?;

    Ok(())
}

/// Reads all checkpoint records from `<project>/.innen/checkpoints.jsonl` in file order.
pub fn load_checkpoints(project_dir: &Path) -> Result<Vec<Checkpoint>, ReadError> {
    let path = checkpoint_file_path(project_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path).map_err(|e| {
        ReadError(format!(
            "failed to open checkpoints file at {}: {e}",
            path.display()
        ))
    })?;

    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line
            .map_err(|e| ReadError(format!("error reading {}:{}: {e}", path.display(), idx + 1)))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cp: Checkpoint = serde_json::from_str(trimmed).map_err(|e| {
            ReadError(format!(
                "invalid checkpoint JSON in {}:{}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        records.push(cp);
    }
    Ok(records)
}

/// Returns the entire history of checkpoints for a specific session ID in chronological order.
pub fn load_history_for_session(
    project_dir: &Path,
    session_id: &str,
) -> Result<Vec<Checkpoint>, ReadError> {
    let all = load_checkpoints(project_dir)?;
    Ok(all
        .into_iter()
        .filter(|c| c.session == session_id)
        .collect())
}

/// Returns the latest checkpoint for a specific session ID.
pub fn latest_checkpoint_for_session(
    project_dir: &Path,
    session_id: &str,
) -> Result<Option<Checkpoint>, ReadError> {
    let history = load_history_for_session(project_dir, session_id)?;
    Ok(history.into_iter().last())
}

/// Returns the latest checkpoint for each session in the project, ordered by updated_at descending.
pub fn latest_checkpoints(project_dir: &Path) -> Result<Vec<Checkpoint>, ReadError> {
    let all = load_checkpoints(project_dir)?;
    let mut latest_map: BTreeMap<String, Checkpoint> = BTreeMap::new();
    for cp in all {
        latest_map.insert(cp.session.clone(), cp);
    }
    let mut list: Vec<Checkpoint> = latest_map.into_values().collect();
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(list)
}

/// Helper to generate a new checkpoint with the current time.
#[allow(clippy::too_many_arguments)]
pub fn new_checkpoint(
    session: String,
    source: Source,
    project: PathBuf,
    status: CheckpointStatus,
    objective: String,
    completed_work: Vec<String>,
    current_evidence: Vec<String>,
    blockers: Vec<String>,
    next_action: String,
    verification: String,
) -> Checkpoint {
    let updated_at = system_time_to_rfc3339(std::time::SystemTime::now());
    Checkpoint {
        session,
        source,
        project,
        status,
        objective,
        completed_work,
        current_evidence,
        blockers,
        next_action,
        verification,
        updated_at,
    }
}

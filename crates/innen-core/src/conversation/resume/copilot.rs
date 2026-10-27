//! Copilot CLI session discovery.
//!
//! Sessions live under `~/.copilot/session-state/<uuid>/`:
//! - `events.jsonl` marks a conversation that actually animated (the remaining
//!   directories are empty scaffolds carrying only workspace.yaml and
//!   checkpoints, and yield no candidate);
//! - `workspace.yaml` records `cwd:` plus product metadata;
//! - `session.shutdown` is an orderly exit, **not** a finished turn, so
//!   discovery keeps every animated session and the evidence layer judges
//!   the transcript tail.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{system_time_to_rfc3339, Candidate};

pub(super) fn scan_copilot_sessions(
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    let all_projects = target_paths.is_empty();
    let mut targets = std::collections::HashSet::<String>::new();
    for target in target_paths {
        targets.insert(target.to_string_lossy().to_string());
        if let Ok(canonical) = target.canonicalize() {
            targets.insert(canonical.to_string_lossy().to_string());
        }
    }

    let sessions = session_dirs(root);
    for dir in sessions {
        let events = dir.join("events.jsonl");
        if !events.is_file() {
            continue;
        }
        let Some(sid) = dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let id = match sources::validate_id(&sid.to_string()) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let project = workspace_cwd(&dir).unwrap_or_else(|| "unknown".into());
        let matched = all_projects || targets.contains(&project);
        if !matched {
            continue;
        }
        let modified = fs::metadata(&events)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map(system_time_to_rfc3339);
        candidates.push(Candidate {
            id,
            source: Source::Copilot,
            modified,
            path: events,
            project: Some(project),
            parent_id: None,
        });
    }
    Ok(())
}

/// Directories under the session-state root that can animate a
/// conversation (the root itself may be a single session directory).
fn session_dirs(root: &Path) -> Vec<PathBuf> {
    if root.join("events.jsonl").is_file() {
        return vec![root.to_path_buf()];
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    dirs
}

/// Extracts the `cwd:` line from workspace.yaml.
fn workspace_cwd(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("workspace.yaml")).ok()?;
    for line in text.lines() {
        // `?` here would abort the loop on the first non-matching line --
        // skip lines that do not carry the key instead.
        if let Some(rest) = line.strip_prefix("cwd:") {
            let rest = rest.trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

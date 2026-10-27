//! Mcode (MiniMax agent runtime) session discovery.
//!
//! The authoritative session metadata lives in
//! `~/.minimax/v2/sqlite/runtime-state.sqlite` (`local_runtime_sessions`):
//! it carries the workspace directory, the title, the runtime status, and
//! `history_relative_dir`, which points at the JSONL transcript under
//! `~/.minimax/v2/sessions/`. No session status marks a completed run
//! (started/idle/aborted/error only), so every in-window session stays a
//! candidate and the unfinished evidence layer classifies the tail instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::Candidate;

pub(super) fn scan_mcode_sessions(
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    let db_path = sources::mcode_runtime_db(root);
    if !db_path.exists() {
        return Ok(());
    }
    // Transcripts root: `sessions/` sits beside the `sqlite/` directory under
    // the runtime data directory; a directory root that carries its own
    // `sessions/` wins so custom store layouts stay addressable.
    let sessions_root = if root.is_dir() && root.join("sessions").is_dir() {
        root.join("sessions")
    } else {
        db_path
            .ancestors()
            .nth(2)
            .map(|p| p.join("sessions"))
            .unwrap_or_else(|| root.to_path_buf())
    };

    let all_projects = target_paths.is_empty();
    let mut targets: HashSet<String> = target_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    for target in target_paths {
        if let Ok(canonical) = target.canonicalize() {
            targets.insert(canonical.to_string_lossy().to_string());
        }
    }

    // archived=0 AND visibility='visible' keep hidden/retired runtimes out.
    // No status filter: there is no completed status to exclude, so
    // discovery keeps every live session and the evidence layer judges.
    let Some(rows) = super::store_query_with_retry(
        &db_path,
        "SELECT json_object('id',session_id,'ws',workspace_dir,'pws',project_workspace_dir,'modified',datetime(updated_at_ms/1000,'unixepoch'),'hist',history_relative_dir,'parent',parent_session_id) FROM local_runtime_sessions WHERE archived = 0 AND visibility = 'visible'",
    ) else {
        return Ok(());
    };

    for row in &rows {
        let Some(sid) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Ok(id) = sources::validate_id(sid) else {
            continue;
        };
        let pws = row.get("pws").and_then(Value::as_str).unwrap_or("");
        let ws = row.get("ws").and_then(Value::as_str).unwrap_or("");
        let matched = all_projects
            || [pws, ws]
                .iter()
                .any(|dir| !dir.is_empty() && targets.contains(*dir));
        if !matched {
            continue;
        }

        let modified = row
            .get("modified")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from);
        let project = if !pws.is_empty() {
            Some(pws.to_string())
        } else if !ws.is_empty() {
            Some(ws.to_string())
        } else {
            Some("unknown".into())
        };

        let id_parent = row
            .get("parent")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(String::from);

        let transcript =
            messages_jsonl_path(&sessions_root, &id, row.get("hist").and_then(Value::as_str));
        let path = transcript.unwrap_or_else(|| db_path.clone());

        candidates.push(Candidate {
            id,
            source: Source::Mcode,
            modified,
            path,
            project,
            parent_id: id_parent,
        });
    }
    Ok(())
}

/// Resolves `messages.jsonl` for a session: the recorded
/// `history_relative_dir` is authoritative; a bounded search for the
/// deterministic `-session_<base64>` directory name is the fallback.
fn messages_jsonl_path(sessions_root: &Path, id: &str, hist: Option<&str>) -> Option<PathBuf> {
    if let Some(hist) = hist {
        if !hist.is_empty() {
            let candidate = sessions_root.join(hist).join("messages.jsonl");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let suffix = sources::mcode_session_dir_suffix(id)?;
    let found = find_session_dir(sessions_root, &suffix, 4)?;
    let candidate = found.join("messages.jsonl");
    candidate.exists().then_some(candidate)
}

fn find_session_dir(root: &Path, suffix: &str, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = entry.file_name();
        if dir_name.to_str().is_some_and(|name| name.ends_with(suffix)) {
            return Some(path);
        }
        if depth > 1 {
            if let Some(found) = find_session_dir(&path, suffix, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

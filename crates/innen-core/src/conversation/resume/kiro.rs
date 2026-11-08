use std::fs;
use std::path::{Path, PathBuf};

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{system_time_to_rfc3339, Candidate};

pub(super) fn scan_kiro_sessions(
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    if !root.is_dir() {
        return Ok(());
    }
    let all_projects = target_paths.is_empty();
    let target_strings: Vec<String> = target_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let mut session_dirs = Vec::new();
    collect_session_dirs(root, &mut session_dirs, 4);
    for dir in session_dirs {
        let messages = dir.join("messages.jsonl");
        let session_json = dir.join("session.json");
        if !messages.exists() && !session_json.exists() {
            continue;
        }
        let sid = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Ok(id) = sources::validate_id(sid) else {
            continue;
        };
        let meta = sources::kiro::session_metadata(&messages);
        let project = meta
            .as_ref()
            .and_then(|m| m.get("workspacePaths"))
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .or_else(|| {
                meta.as_ref()
                    .and_then(|m| m.get("rootPaths"))
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
            })
            .map(str::to_string)
            .unwrap_or_else(|| "unknown".into());
        let matched = all_projects
            || target_strings
                .iter()
                .any(|ts| project.contains(ts) || ts.contains(&project));
        if !matched {
            continue;
        }
        let modified = meta
            .as_ref()
            .and_then(|m| m.get("lastModifiedAt").and_then(|v| v.as_str()))
            .map(str::to_string)
            .or_else(|| {
                fs::metadata(if messages.exists() {
                    &messages
                } else {
                    &session_json
                })
                .ok()
                .and_then(|m| m.modified().ok())
                .map(system_time_to_rfc3339)
            });
        candidates.push(Candidate {
            id,
            source: Source::Kiro,
            modified,
            path: if messages.exists() {
                messages
            } else {
                session_json
            },
            project: Some(project),
            parent_id: None,
        });
    }
    Ok(())
}

fn collect_session_dirs(root: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("sess_") {
            out.push(path);
            continue;
        }
        if depth > 0 {
            collect_session_dirs(&path, out, depth - 1);
        }
    }
}

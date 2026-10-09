use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{collect_files, latest_event_timestamp, system_time_to_rfc3339, Candidate};

pub(super) fn scan_codex_sessions(
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

    let mut files = Vec::new();
    collect_files(root, "jsonl", 6, &mut files);

    for path in files {
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(file);
        let mut line0 = String::new();
        if reader.read_line(&mut line0).is_err() || line0.is_empty() {
            continue;
        }
        if !line0.contains("session_meta") {
            continue;
        }
        if !all_projects && !target_strings.iter().any(|ts| line0.contains(ts)) {
            continue;
        }
        if let Ok(meta) = serde_json::from_str::<Value>(&line0) {
            if let Some(payload) = meta.get("payload") {
                let cwd_str = payload.get("cwd").and_then(Value::as_str);
                let matched = if all_projects {
                    true
                } else if let Some(cwd) = cwd_str {
                    let cwd_path = PathBuf::from(cwd);
                    let canonical_cwd =
                        cwd_path.canonicalize().unwrap_or_else(|_| cwd_path.clone());
                    target_paths
                        .iter()
                        .any(|tp| tp == &canonical_cwd || tp == &cwd_path)
                } else {
                    false
                };
                if matched {
                    let sid = payload
                        .get("id")
                        .or_else(|| payload.get("session_id"))
                        .and_then(Value::as_str);
                    if let Some(sid) = sid {
                        if let Ok(id) = sources::validate_id(sid) {
                            let modified = latest_event_timestamp(&path).or_else(|| {
                                fs::metadata(&path)
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .map(system_time_to_rfc3339)
                            });
                            let project = cwd_str
                                .map(|s| s.to_string())
                                .or_else(|| Some("unknown".into()));
                            // Codex records a subagent's parent under
                            // source.subagent.thread_spawn; keep the older flat keys.
                            let parent_id = payload
                                .pointer("/source/subagent/thread_spawn/parent_thread_id")
                                .or_else(|| payload.get("parent_thread_id"))
                                .or_else(|| payload.get("parent_session_id"))
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            candidates.push(Candidate {
                                id,
                                source: Source::Codex,
                                modified,
                                path,
                                project,
                                parent_id,
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

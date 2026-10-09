use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{latest_event_timestamp, system_time_to_rfc3339, Candidate};

fn extract_antigravity_project_from_uris(uris: &str) -> Option<String> {
    if uris.trim().is_empty() {
        return None;
    }
    for part in uris.split([',', ' ', '\n']) {
        let trimmed = part.trim().trim_matches('"');
        if let Some(rest) = trimmed.strip_prefix("file://") {
            let decoded = rest.replace("%20", " ").replace("%2F", "/");
            if !decoded.is_empty() {
                return Some(decoded);
            }
        } else if trimmed.starts_with('/') {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn extract_antigravity_project_from_lines(lines: &[String]) -> Option<String> {
    let mut explicit_unknown = false;
    for line in lines {
        if line.contains("The user does not have any active workspace") {
            explicit_unknown = true;
        }
        for prefix in ["Workspace: ", "workspace: ", "Work on project in "] {
            if let Some(idx) = line.find(prefix) {
                let rest = &line[idx + prefix.len()..];
                let end = rest
                    .find(['\n', '\r', '"', '`', ' ', '<', '\\'])
                    .unwrap_or(rest.len());
                let candidate = rest[..end].trim();
                if candidate.starts_with('/') {
                    return Some(candidate.to_string());
                }
            }
        }
        for key in [
            "\"Cwd\":\"",
            "\"Cwd\": \"",
            "\"SearchDirectory\":\"",
            "\"SearchDirectory\": \"",
            "\"DirectoryPath\":\"",
            "\"DirectoryPath\": \"",
        ] {
            if let Some(idx) = line.find(key) {
                let rest = &line[idx + key.len()..];
                if let Some(end) = rest.find('"') {
                    let p = rest[..end].trim();
                    if p.starts_with('/') {
                        return Some(p.to_string());
                    }
                }
            }
        }
    }
    if explicit_unknown {
        Some("unknown".into())
    } else {
        None
    }
}

pub(super) fn scan_antigravity_sessions(
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    let all_projects = target_paths.is_empty();
    let target_strings: Vec<String> = target_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    // 1. Try conversation_summaries.db in parent or root
    let db_paths = [
        root.parent().map(|p| p.join("conversation_summaries.db")),
        Some(root.join("conversation_summaries.db")),
    ];
    for db_path in db_paths.into_iter().flatten() {
        if db_path.exists() {
            if let Ok(rows) = sources::sqlite(
                &db_path,
                "SELECT json_object('id',conversation_id,'modified',last_modified_time,'uris',workspace_uris) FROM conversation_summaries",
            ) {
                for row in rows {
                    let uris = row.get("uris").and_then(Value::as_str).unwrap_or("");
                    let matched = if all_projects {
                        true
                    } else {
                        target_strings.iter().any(|ts| {
                            uris.contains(ts) || uris.contains(&ts.replace('/', "%2F"))
                        })
                    };
                    if matched {
                        if let Some(sid) = row.get("id").and_then(Value::as_str) {
                            if let Ok(id) = sources::validate_id(sid) {
                                if !candidates
                                    .iter()
                                    .any(|c| c.id == id && c.source == Source::Antigravity)
                                {
                                    let modified = row
                                        .get("modified")
                                        .and_then(Value::as_str)
                                        .map(String::from);
                                    let path =
                                        root.join(&id).join(".system_generated/logs/transcript.jsonl");
                                    let project = extract_antigravity_project_from_uris(uris)
                                        .or_else(|| {
                                            if all_projects {
                                                Some("unknown".into())
                                            } else {
                                                target_paths.first().map(|p| p.to_string_lossy().to_string())
                                            }
                                        });
                                    candidates.push(Candidate {
                                        id,
                                        source: Source::Antigravity,
                                        modified,
                                        path,
                                        project,
                                        parent_id: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Scan brain directories under root
    if root.is_dir() {
        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let mut transcript_path = None;
            let mut session_id = None;

            if path.is_dir() {
                let candidate1 = path.join(".system_generated/logs/transcript.jsonl");
                let candidate2 = path.join("transcript.jsonl");
                if candidate1.exists() {
                    transcript_path = Some(candidate1);
                    session_id = path.file_name().and_then(|n| n.to_str()).map(String::from);
                } else if candidate2.exists() {
                    transcript_path = Some(candidate2);
                    session_id = path.file_name().and_then(|n| n.to_str()).map(String::from);
                }
            } else if path.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
                transcript_path = Some(path.clone());
                session_id = path.file_stem().and_then(|s| s.to_str()).map(String::from);
            }

            let Some(transcript) = transcript_path else {
                continue;
            };
            let Some(sid) = session_id else {
                continue;
            };
            let Ok(id) = sources::validate_id(&sid) else {
                continue;
            };
            if candidates
                .iter()
                .any(|c| c.id == id && c.source == Source::Antigravity)
            {
                continue;
            }

            if let Ok(file) = File::open(&transcript) {
                let reader = BufReader::new(file);
                let lines_sample: Vec<String> =
                    reader.lines().take(50).map_while(Result::ok).collect();
                let matched = if all_projects {
                    true
                } else {
                    target_strings.iter().any(|ts| {
                        lines_sample
                            .iter()
                            .any(|line| line.contains(ts) || line.contains(&ts.replace('/', "%2F")))
                    })
                };
                if matched {
                    let modified = latest_event_timestamp(&transcript).or_else(|| {
                        fs::metadata(&transcript)
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(system_time_to_rfc3339)
                    });
                    let detected_proj = extract_antigravity_project_from_lines(&lines_sample);
                    let project = if all_projects {
                        detected_proj.or_else(|| Some("unknown".into()))
                    } else {
                        detected_proj.or_else(|| {
                            target_paths
                                .first()
                                .map(|p| p.to_string_lossy().to_string())
                        })
                    };
                    candidates.push(Candidate {
                        id,
                        source: Source::Antigravity,
                        modified,
                        path: transcript,
                        project,
                        parent_id: None,
                    });
                }
            }
        }
    }

    // 3. The current Antigravity CLI keeps the authoritative trajectory in
    // sibling `conversations/<uuid>.db` files.  Some sessions have no readable
    // transcript under brain/, so inventory them as native candidates instead
    // of silently under-counting the store.  The dialogue reader remains
    // fail-closed for proprietary DB-only sessions; retention can still hash
    // the complete DB + brain bundle and report the missing extraction proof.
    if root.file_name().is_some_and(|name| name == "brain") {
        if let Some(store_root) = root.parent() {
            let conversations = store_root.join("conversations");
            if let Ok(entries) = fs::read_dir(&conversations) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() || path.extension().is_none_or(|ext| ext != "db") {
                        continue;
                    }
                    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                        continue;
                    };
                    let Ok(id) = sources::validate_id(stem) else {
                        continue;
                    };
                    if candidates.iter().any(|candidate| {
                        candidate.id == id && candidate.source == Source::Antigravity
                    }) {
                        continue;
                    }
                    let modified = entry
                        .metadata()
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .map(system_time_to_rfc3339);
                    candidates.push(Candidate {
                        id,
                        source: Source::Antigravity,
                        modified,
                        path,
                        project: Some("unknown".into()),
                        parent_id: None,
                    });
                }
            }
        }
    }
    Ok(())
}

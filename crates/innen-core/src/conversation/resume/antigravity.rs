use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{latest_event_timestamp, system_time_to_rfc3339, Candidate};

/// A modification time the retention gate may act on.
///
/// A timestamp before 1970 is not a real modification time: Antigravity writes
/// the zero time into `last_modified_time` for conversations it never
/// timestamped, and a gate that reads that as "very old" would delete a
/// session whose age nobody established.
fn is_trustworthy_stamp(stamp: &str) -> bool {
    stamp
        .get(..4)
        .and_then(|year| year.parse::<u16>().ok())
        .is_some_and(|year| year >= 1970)
}

/// The age of the native bytes of one Antigravity conversation.
///
/// The summary database sometimes leaves `last_modified_time` unset. The bytes
/// on disk still know how old they are, and the retention gate is about those
/// bytes, so their own modification time answers the question the missing
/// field left open. The newest of the two wins: a byte that changed recently
/// keeps its conversation out of reach.
fn native_bytes_modified(root: &Path, id: &str) -> Option<String> {
    let conversation_db = root
        .parent()
        .map(|store| store.join("conversations").join(format!("{id}.db")));
    [Some(root.join(id)), conversation_db]
        .into_iter()
        .flatten()
        .filter_map(|path| fs::metadata(path).ok())
        .filter_map(|meta| meta.modified().ok())
        .max()
        .map(system_time_to_rfc3339)
}

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
                                        .filter(|stamp| is_trustworthy_stamp(stamp))
                                        .map(String::from)
                                        .or_else(|| native_bytes_modified(root, &id));
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

#[cfg(test)]
mod tests {
    use super::{is_trustworthy_stamp, native_bytes_modified};
    use std::fs;
    use std::path::Path;

    /// Antigravity writes the zero time into `last_modified_time` for
    /// conversations it never timestamped. That is not a real age, so the gate
    /// must not read it as one.
    #[test]
    fn the_zero_time_is_not_a_trustworthy_age() {
        assert!(!is_trustworthy_stamp("0001-01-01 00:00:00+00:00"));
        assert!(is_trustworthy_stamp("2026-08-27 14:38:00+00:00"));
    }

    /// When the summary row has no usable timestamp, the native bytes answer
    /// it: their own modification time is the age the gate acts on.
    #[test]
    fn native_bytes_supply_the_age_a_missing_timestamp_leaves_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("antigravity-cli");
        fs::create_dir_all(store.join("brain").join("session-id")).expect("mkdir brain");
        fs::create_dir_all(store.join("conversations")).expect("mkdir conversations");
        fs::write(store.join("conversations").join("session-id.db"), b"bytes").expect("write");

        let modified = native_bytes_modified(&store.join("brain"), "session-id")
            .expect("the native bytes carry their own age");
        assert!(
            is_trustworthy_stamp(&modified),
            "the fallback must be a real timestamp, got {modified}"
        );
    }

    /// The newest native byte wins: a conversation whose database changed
    /// recently must not look stale because its brain directory did not.
    #[test]
    fn the_newest_native_byte_decides_the_age() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("antigravity-cli");
        let brain = store.join("brain").join("session-id");
        fs::create_dir_all(&brain).expect("mkdir brain");
        fs::create_dir_all(store.join("conversations")).expect("mkdir conversations");
        fs::write(store.join("conversations").join("session-id.db"), b"bytes").expect("write");

        let stale = native_bytes_modified(&store.join("brain"), "session-id").expect("age");
        // Touch only the conversation database, then read the age again.
        let database = store.join("conversations").join("session-id.db");
        let touched = std::time::SystemTime::now() + std::time::Duration::from_secs(3_600);
        fs::File::options()
            .write(true)
            .open(&database)
            .expect("open")
            .set_times(fs::FileTimes::new().set_modified(touched))
            .expect("set mtime");

        let fresh = native_bytes_modified(&store.join("brain"), "session-id").expect("age");
        assert_ne!(stale, fresh, "the newest native byte must decide the age");
        assert!(
            fresh > stale,
            "the touched database must produce the newer age: {fresh} vs {stale}"
        );
        assert!(Path::new(&database).exists());
    }
}

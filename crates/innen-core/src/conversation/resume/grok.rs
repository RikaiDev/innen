//! Grok CLI session discovery.
//!
//! A session directory `~/.grok/sessions/<percent-encoded project>/<uuid>/`
//! is a candidate when the documented conversation file `chat_history.jsonl`
//! exists. The sibling `events.jsonl` runtime stream belongs to the evidence
//! layer only, and the candidate timestamp is the newest stream mtime in the
//! session directory so ordering survives a tail that advanced only one file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{system_time_to_rfc3339, Candidate};

/// The stream files grok writes into a session directory. Discovery anchors
/// on the documented conversation stream; the timestamp covers all of them
/// because any one can be the last thing the runtime advanced.
const GROK_STREAMS: [&str; 3] = ["chat_history.jsonl", "events.jsonl", "updates.jsonl"];

pub(super) fn scan_grok_sessions(
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
    let Ok(projects) = fs::read_dir(root) else {
        return Ok(());
    };
    for project_dir in projects.flatten() {
        let project_path = project_dir.path();
        if !project_path.is_dir() {
            continue;
        }
        let Some(dirname) = project_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let project = percent_decode(dirname);
        if !all_projects && !targets.contains(&project) {
            continue;
        }
        let Ok(sessions) = fs::read_dir(&project_path) else {
            continue;
        };
        for session in sessions.flatten() {
            let session_dir = session.path();
            if !session_dir.is_dir() {
                continue;
            }
            // The documented conversation stream is the candidate anchor;
            // running ACP traffic alone does not create a candidate.
            let history = session_dir.join("chat_history.jsonl");
            if !history.is_file() {
                continue;
            }
            let Some(os_id) = session_dir.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let id = match sources::validate_id(os_id) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let modified = newest_stream_mtime(&session_dir).map(system_time_to_rfc3339);
            candidates.push(Candidate {
                id,
                source: Source::Grok,
                modified,
                path: history,
                project: Some(project.clone()),
                parent_id: None,
            });
        }
    }
    Ok(())
}

/// Newest mtime among the streams grok writes into the session directory.
fn newest_stream_mtime(session_dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    for name in GROK_STREAMS {
        if let Ok(modified) = fs::metadata(session_dir.join(name)).and_then(|m| m.modified()) {
            if newest.is_none_or(|prior| modified > prior) {
                newest = Some(modified);
            }
        }
    }
    newest
}

/// Decodes percent-encoding (`%2F`, `%20`, `%3A`...) used in Grok project
/// directory names; leaves plain bytes untouched.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let hex = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                decoded.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}

//! Kiro CLI/Desktop session storage.
//!
//! `~/.kiro/sessions` contains per-workspace session directories:
//! `<workspace>/<sess_...>/{session.json,messages.jsonl,...}` and a root
//! `session-index/*.jsonl`. The primary transcript is `messages.jsonl`
//! (one JSON object per line with `payload.type` / `payload.content`).

use super::{err, ReadError};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn load_messages(path: &Path) -> Result<Vec<Value>, ReadError> {
    let transcript = transcript_path(path);
    let file = File::open(&transcript).map_err(|e| err(&transcript, e))?;
    let mut out = Vec::new();
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| err(&transcript, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(&line).map_err(|e| {
            ReadError(format!(
                "invalid transcript JSON at {}:{}: {e}",
                transcript.display(),
                line_no + 1
            ))
        })?;
        out.push(event);
    }
    Ok(out)
}

pub(crate) fn transcript_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        let messages = path.join("messages.jsonl");
        if messages.exists() {
            return messages;
        }
    }
    if path.file_name().is_some_and(|n| n == "messages.jsonl") {
        return path.to_owned();
    }
    path.to_owned()
}

pub(crate) fn session_metadata(path: &Path) -> Option<Value> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    let text = std::fs::read_to_string(dir.join("session.json")).ok()?;
    serde_json::from_str(&text).ok()
}

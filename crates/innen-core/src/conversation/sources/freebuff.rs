//! Freebuff (Codebuff) session storage.
//!
//! Two formats:
//! - CLI: `~/.config/manicode/projects/<project>/chats/<timestamp>/chat-messages.json`
//!   (JSON array of messages with `variant`/`blocks`), plus `chat-meta.json`.
//! - Desktop: `~/.config/freebuff-desktop/projects/<project-dir>/desktop-v2.db`
//!   SQLite with `threads` and `messages` tables; `parts_json` holds structured parts.

use super::{err, sqlite, Located, ReadError};
use serde_json::Value;
use std::path::Path;

/// Load CLI chat-messages.json as raw event objects.
pub fn load_cli_messages(path: &Path) -> Result<Vec<Value>, ReadError> {
    let text = std::fs::read_to_string(path).map_err(|e| err(path, e))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        ReadError(format!(
            "invalid freebuff chat-messages.json at {}: {e}",
            path.display()
        ))
    })?;
    match value {
        Value::Array(arr) => Ok(arr),
        _ => Err(ReadError(format!(
            "expected JSON array in freebuff chat-messages.json: {}",
            path.display()
        ))),
    }
}

/// Load Desktop messages from desktop-v2.db for a thread.
pub fn load_desktop_messages(path: &Path, thread_id: &str) -> Result<Vec<Value>, ReadError> {
    let db = if path.is_dir() {
        path.join("desktop-v2.db")
    } else {
        path.to_path_buf()
    };
    let escaped = thread_id.replace('\'', "''");
    let sql = format!(
        "SELECT json_object(\
            'seq', seq, \
            'thread_id', thread_id, \
            'role', role, \
            'parts_json', json(parts_json), \
            'attachments_json', json(attachments_json), \
            'metrics_json', json(metrics_json), \
            'ts', ts, \
            'request_id', request_id, \
            'origin', origin, \
            'steering', steering\
        ) FROM messages WHERE thread_id='{}' ORDER BY ts, seq",
        escaped
    );
    sqlite(&db, &sql)
}

/// Whether this path is a Freebuff Desktop database.
pub(crate) fn is_desktop_db(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "desktop-v2.db")
        || (path.is_dir() && path.join("desktop-v2.db").exists())
}

/// Load all events for a Freebuff session (CLI JSON array or Desktop DB rows).
pub fn load_messages(located: &Located, id: &str) -> Result<Vec<Value>, ReadError> {
    let path = &located.path;
    if is_desktop_db(path) {
        load_desktop_messages(path, id)
    } else if path.file_name().is_some_and(|n| n == "chat-messages.json") {
        load_cli_messages(path)
    } else {
        // Try CLI directory
        let cli_file = if path.is_dir() {
            path.join("chat-messages.json")
        } else {
            path.to_path_buf()
        };
        if cli_file.exists() {
            load_cli_messages(&cli_file)
        } else {
            Err(ReadError(format!(
                "unsupported freebuff session path: {}",
                path.display()
            )))
        }
    }
}

//! Locating one session by identity inside a store.
//!
//! The store layout differs per tool, so the search rules live here once
//! instead of in every consumer.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    default_stores, err, mcode_runtime_db, mcode_session_dir_suffix, session_table, sqlite,
    Located, ReadError, Source,
};

pub fn locate(root: Option<&Path>, source: &str, id: &str) -> Result<Located, ReadError> {
    let filter = if source == "auto" {
        None
    } else {
        Some(Source::parse(source)?)
    };
    let stores = if let Some(root) = root {
        vec![(
            filter.ok_or_else(|| ReadError("--source-root requires --source <tool>".into()))?,
            root.to_owned(),
        )]
    } else {
        default_stores()?
    };
    let mut found: Vec<Located> = Vec::new();
    for (kind, root) in stores {
        if filter.is_some_and(|s| s != kind) {
            continue;
        }
        let candidates = if kind == Source::Antigravity {
            let mut list = vec![
                root.join(id)
                    .join(".system_generated/logs/transcript.jsonl"),
                root.join(id).join("transcript.jsonl"),
                root.join(format!("{id}.jsonl")),
            ];
            let found_direct = list.iter().any(|p| p.try_exists().unwrap_or(false));
            if !found_direct {
                let _ = walk(&root, kind, id, 6, &mut list);
            }
            list
        } else if kind == Source::Copilot {
            vec![root.join(id).join("events.jsonl")]
        } else if kind == Source::Opencode {
            let path = if root.is_dir() {
                root.join("opencode.db")
            } else {
                root
            };
            if path.try_exists().map_err(|e| err(&path, e))?
                && !sqlite(
                    &path,
                    &format!(
                        "SELECT json_object('id',id) FROM {} WHERE id='{}' LIMIT 1",
                        session_table(&path)?,
                        id
                    ),
                )?
                .is_empty()
            {
                vec![path]
            } else {
                Vec::new()
            }
        } else if kind == Source::Mcode {
            let db = mcode_runtime_db(&root);
            let sessions_root = if root.is_dir() && root.join("sessions").is_dir() {
                root.join("sessions")
            } else {
                db.ancestors()
                    .nth(2)
                    .map(|p| p.join("sessions"))
                    .unwrap_or_else(|| root.to_path_buf())
            };
            let mut list = Vec::new();
            if db.exists() {
                // history_relative_dir is the authoritative transcript pointer.
                if let Ok(rows) = sqlite(
                    &db,
                    &format!(
                        "SELECT json_object('hist',history_relative_dir) FROM local_runtime_sessions WHERE session_id='{}' LIMIT 1",
                        id
                    ),
                ) {
                    for row in rows {
                        if let Some(hist) = row.get("hist").and_then(Value::as_str) {
                            if !hist.is_empty() {
                                list.push(sessions_root.join(hist).join("messages.jsonl"));
                            }
                        }
                    }
                }
            }
            if list.is_empty() {
                let _ = walk(&sessions_root, kind, id, 4, &mut list);
            }
            list
        } else {
            let mut candidates = Vec::new();
            walk(&root, kind, id, 6, &mut candidates)?;
            candidates
        };
        for path in candidates {
            match path.canonicalize() {
                Ok(path) => {
                    if !found.iter().any(|f| f.path == path && f.source == kind) {
                        found.push(Located { source: kind, path });
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(&path, e)),
            }
        }
    }
    match found.len() {
        0 => Err(ReadError(format!("conversation not found: {id} (source {source}); use --source <tool> --source-root <store> for relocated data; no ingest required"))),
        1 => Ok(found.remove(0)),
        _ => Err(ReadError(format!("ambiguous conversation: {id}; select --source and --source-root from: {}", found.iter().map(|f| format!("{:?}:{}", f.source, f.path.display())).collect::<Vec<_>>().join(", ")))),
    }
}
fn walk(
    root: &Path,
    source: Source,
    id: &str,
    depth: usize,
    found: &mut Vec<PathBuf>,
) -> Result<(), ReadError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(err(root, e)),
    };
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(root, e))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| err(&path, e))?;
        if ty.is_dir() && depth > 0 {
            walk(&path, source, id, depth - 1, found)?;
        }
        if !ty.is_file() {
            continue;
        }
        let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
        // Codex rotates older rollouts into `<id>.jsonl.zst`; the stem of such a
        // file still ends with the session id, so the match must look past the
        // extra compression suffix.
        let codex_stem = stem.strip_suffix(".jsonl").unwrap_or(stem);
        let matched = match source {
            Source::Codex => {
                (ext == "jsonl" || ext == "zst")
                    && (codex_stem == id || codex_stem.ends_with(&format!("-{id}")))
            }
            Source::Claude | Source::Cursor | Source::Qwen => ext == "jsonl" && stem == id,
            Source::Mcode => {
                ext == "jsonl"
                    && stem == "messages"
                    && mcode_session_dir_suffix(id).is_some_and(|suffix| {
                        path.parent()
                            .and_then(Path::file_name)
                            .and_then(|n| n.to_str())
                            .is_some_and(|dir| dir.ends_with(&suffix))
                    })
            }
            Source::Antigravity => {
                (ext == "jsonl" && stem == id)
                    || (ext == "jsonl"
                        && stem == "transcript"
                        && path
                            .ancestors()
                            .nth(3)
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n == id))
                    || (ext == "jsonl"
                        && stem == "transcript"
                        && path
                            .parent()
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n == id))
            }
            Source::Vscode => {
                ext == "json"
                    && stem == id
                    && path.parent().is_some_and(|p| p.ends_with("chatSessions"))
            }
            Source::Grok => {
                path.file_name().is_some_and(|n| n == "chat_history.jsonl")
                    && path
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|n| n == id)
            }
            Source::Gemini => {
                // Gemini filenames may contain only the first eight ID chars;
                // always verify the complete sessionId in file metadata.
                if matches!(ext, "json" | "jsonl")
                    && stem.starts_with("session-")
                    && stem.contains(&id[..8.min(id.len())])
                {
                    let text = fs::read_to_string(&path).map_err(|e| err(&path, e))?;
                    let first = serde_json::from_str::<Value>(&text)
                        .or_else(|_| serde_json::from_str(text.lines().next().unwrap_or("")))
                        .map_err(|e| err(&path, e))?;
                    first.get("sessionId").and_then(Value::as_str) == Some(id)
                } else {
                    false
                }
            }
            _ => false,
        };
        if matched {
            found.push(path);
        }
    }
    Ok(())
}

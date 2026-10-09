use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::Candidate;

pub(super) fn scan_opencode_sessions(
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    let db_path = if root.is_dir() {
        root.join("opencode.db")
    } else {
        root.to_path_buf()
    };
    if !db_path.exists() {
        return Ok(());
    }
    let all_projects = target_paths.is_empty();
    let target_strings: Vec<String> = target_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if let Ok(rows) = sources::sqlite(
        &db_path,
        &format!(
                            "SELECT json_object('id',id,'directory',directory,'modified',datetime(time_updated/1000,'unixepoch')) FROM {}",
                            sources::session_table(&db_path)?
                        ),
    ) {
        for row in rows {
            let dir = row.get("directory").and_then(Value::as_str).unwrap_or("");
            let matched = if all_projects {
                true
            } else {
                target_strings.iter().any(|ts| ts == dir)
            };
            if matched {
                if let Some(sid) = row.get("id").and_then(Value::as_str) {
                    if let Ok(id) = sources::validate_id(sid) {
                        let modified =
                            row.get("modified").and_then(Value::as_str).map(String::from);
                        let project = if dir.is_empty() {
                            Some("unknown".into())
                        } else {
                            Some(dir.to_string())
                        };
                        candidates.push(Candidate {
                            id,
                            source: Source::Opencode,
                            modified,
                            path: db_path.clone(),
                            project,
                            parent_id: None,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

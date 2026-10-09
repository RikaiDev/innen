use std::fs;
use std::path::{Path, PathBuf};

use super::super::sources::{self, Source};
use super::super::ReadError;
use super::{latest_event_timestamp, system_time_to_rfc3339, Candidate};

pub(super) fn scan_dash_named_projects(
    source: Source,
    root: &Path,
    target_paths: &[PathBuf],
    candidates: &mut Vec<Candidate>,
) -> Result<(), ReadError> {
    if !root.is_dir() {
        return Ok(());
    }
    let all_projects = target_paths.is_empty();
    if all_projects {
        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            let proj_dir = entry.path();
            if !proj_dir.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !dir_name.starts_with('-') {
                continue;
            }
            let reconstructed = format!("/{}", dir_name.trim_start_matches('-').replace('-', "/"));
            let sub_entries = match fs::read_dir(&proj_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for sub in sub_entries.flatten() {
                let path = sub.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    if let Ok(id) = sources::validate_id(stem) {
                        let modified = latest_event_timestamp(&path).or_else(|| {
                            sub.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .map(system_time_to_rfc3339)
                        });
                        candidates.push(Candidate {
                            id,
                            source,
                            modified,
                            path,
                            project: Some(reconstructed.clone()),
                            parent_id: None,
                        });
                    }
                }
            }
        }
        return Ok(());
    }
    for target in target_paths {
        let dir_name = format!(
            "-{}",
            target
                .to_string_lossy()
                .trim_start_matches('/')
                .replace('/', "-")
        );
        let proj_dir = root.join(&dir_name);
        if !proj_dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(&proj_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Ok(id) = sources::validate_id(stem) {
                    let modified = latest_event_timestamp(&path).or_else(|| {
                        entry
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(system_time_to_rfc3339)
                    });
                    candidates.push(Candidate {
                        id,
                        source,
                        modified,
                        path,
                        project: Some(target.to_string_lossy().to_string()),
                        parent_id: None,
                    });
                }
            }
        }
    }
    Ok(())
}

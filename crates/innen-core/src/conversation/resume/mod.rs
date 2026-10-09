//! Project candidate discovery and resume target resolution.
//! Inspects available project/session metadata across standard local stores
//! without inventing semantic summaries or making model calls.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sources::{self, Source};
use super::ReadError;

mod antigravity;
mod codex;
mod dash;
mod opencode;
mod time;

use antigravity::scan_antigravity_sessions;
use codex::scan_codex_sessions;
use dash::scan_dash_named_projects;
use opencode::scan_opencode_sessions;

pub(crate) use time::system_time_to_rfc3339;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub source: Source,
    pub modified: Option<String>,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResumeTarget {
    Unambiguous(Candidate),
    Ambiguous {
        project: PathBuf,
        candidates: Vec<Candidate>,
    },
    NotFound {
        project: PathBuf,
    },
}

impl ResumeTarget {
    pub fn unambiguous(&self) -> Option<&Candidate> {
        match self {
            Self::Unambiguous(c) => Some(c),
            _ => None,
        }
    }
}

pub fn resolve_resume_target(
    project_dir: &Path,
    source_filter: &str,
    source_root: Option<&Path>,
) -> Result<ResumeTarget, ReadError> {
    let filter = if source_filter == "auto" {
        None
    } else {
        Some(Source::parse(source_filter)?)
    };
    let candidates = find_project_candidates(project_dir, filter, source_root)?;
    let canonical = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    match candidates.len() {
        0 => Ok(ResumeTarget::NotFound { project: canonical }),
        1 => Ok(ResumeTarget::Unambiguous(
            candidates.into_iter().next().unwrap(),
        )),
        _ => Ok(ResumeTarget::Ambiguous {
            project: canonical,
            candidates,
        }),
    }
}

pub fn find_project_candidates(
    project_dir: &Path,
    source_filter: Option<Source>,
    source_root: Option<&Path>,
) -> Result<Vec<Candidate>, ReadError> {
    let mut candidate_paths = Vec::new();
    let raw = project_dir.to_path_buf();
    candidate_paths.push(raw.clone());
    if let Ok(stripped) = raw.strip_prefix("/private") {
        let non_private = PathBuf::from("/").join(stripped);
        if !candidate_paths.contains(&non_private) {
            candidate_paths.push(non_private);
        }
    }
    if let Ok(canonical) = project_dir.canonicalize() {
        if !candidate_paths.contains(&canonical) {
            candidate_paths.push(canonical.clone());
        }
        if let Ok(stripped) = canonical.strip_prefix("/private") {
            let non_private = PathBuf::from("/").join(stripped);
            if !candidate_paths.contains(&non_private) {
                candidate_paths.push(non_private);
            }
        }
    }

    // Also check enclosing git repository root if available
    let mut curr = project_dir;
    while let Some(parent) = curr.parent() {
        if parent.join(".git").exists() {
            let p_raw = parent.to_path_buf();
            if !candidate_paths.contains(&p_raw) {
                candidate_paths.push(p_raw.clone());
            }
            if let Ok(stripped) = p_raw.strip_prefix("/private") {
                let non_private = PathBuf::from("/").join(stripped);
                if !candidate_paths.contains(&non_private) {
                    candidate_paths.push(non_private);
                }
            }
            if let Ok(c) = parent.canonicalize() {
                if !candidate_paths.contains(&c) {
                    candidate_paths.push(c.clone());
                }
                if let Ok(stripped) = c.strip_prefix("/private") {
                    let non_private = PathBuf::from("/").join(stripped);
                    if !candidate_paths.contains(&non_private) {
                        candidate_paths.push(non_private);
                    }
                }
            }
            break;
        }
        curr = parent;
    }

    let stores = if let Some(root) = source_root {
        let s = source_filter
            .ok_or_else(|| ReadError("--source-root requires --source <tool>".into()))?;
        vec![(s, root.to_path_buf())]
    } else {
        sources::default_stores()?
    };

    let mut candidates = Vec::new();
    for (source, root) in stores {
        if source_filter.is_some_and(|s| s != source) {
            continue;
        }
        match source {
            Source::Claude | Source::Qwen => {
                scan_dash_named_projects(source, &root, &candidate_paths, &mut candidates)?;
            }
            Source::Codex => {
                scan_codex_sessions(&root, &candidate_paths, &mut candidates)?;
            }
            Source::Antigravity => {
                scan_antigravity_sessions(&root, &candidate_paths, &mut candidates)?;
            }
            Source::Opencode => {
                scan_opencode_sessions(&root, &candidate_paths, &mut candidates)?;
            }
            _ => {}
        }
    }

    // Sort by modified descending (newest first)
    Ok(deduplicate_candidates(candidates))
}

pub fn find_all_candidates(
    source_filter: Option<Source>,
    source_root: Option<&Path>,
) -> Result<Vec<Candidate>, ReadError> {
    let stores = if let Some(root) = source_root {
        let s = source_filter
            .ok_or_else(|| ReadError("--source-root requires --source <tool>".into()))?;
        vec![(s, root.to_path_buf())]
    } else {
        sources::default_stores()?
    };

    let mut candidates = Vec::new();
    let empty_targets: [PathBuf; 0] = [];
    for (source, root) in stores {
        if source_filter.is_some_and(|s| s != source) {
            continue;
        }
        match source {
            Source::Claude | Source::Qwen => {
                scan_dash_named_projects(source, &root, &empty_targets, &mut candidates)?;
            }
            Source::Codex => {
                scan_codex_sessions(&root, &empty_targets, &mut candidates)?;
            }
            Source::Antigravity => {
                scan_antigravity_sessions(&root, &empty_targets, &mut candidates)?;
            }
            Source::Opencode => {
                scan_opencode_sessions(&root, &empty_targets, &mut candidates)?;
            }
            _ => {}
        }
    }

    Ok(deduplicate_candidates(candidates))
}

fn collect_files(root: &Path, ext_filter: &str, depth: usize, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth > 0 {
            collect_files(&path, ext_filter, depth - 1, files);
        } else if path.is_file() && path.extension().is_some_and(|e| e == ext_filter) {
            files.push(path);
        }
    }
}

/// Return the latest timestamp recorded by the source itself. Filename and
/// filesystem mtime are only fallbacks for transcripts with no event time.
/// This keeps candidate ordering correct after copied or concurrently-written
/// files, while retaining bounded memory (one timestamp at a time).
fn latest_event_timestamp(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let start = size.saturating_sub(1024 * 1024);
    let mut file = file;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(size - start).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut latest: Option<String> = None;
    for line in text.lines().skip(usize::from(start > 0)) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for key in ["timestamp", "created_at", "updated_at", "time_created"] {
            if let Some(value) = event.get(key).and_then(Value::as_str) {
                if latest.as_deref().is_none_or(|current| value > current) {
                    latest = Some(value.to_owned());
                }
            }
        }
    }
    latest
}

fn deduplicate_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut selected = std::collections::HashMap::<(Source, String), Candidate>::new();
    for candidate in candidates.drain(..) {
        let key = (candidate.source, candidate.id.clone());
        let replace = selected.get(&key).is_none_or(|current| {
            candidate.modified > current.modified
                || (candidate.modified == current.modified && candidate.path < current.path)
        });
        if replace {
            selected.insert(key, candidate);
        }
    }
    let mut out: Vec<_> = selected.into_values().collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    out
}

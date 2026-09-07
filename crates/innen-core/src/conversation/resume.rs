//! Project candidate discovery and resume target resolution.
//! Inspects available project/session metadata across standard local stores
//! without inventing semantic summaries or making model calls.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sources::{self, Source};
use super::ReadError;

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

fn scan_dash_named_projects(
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

fn scan_codex_sessions(
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
                            let parent_id = payload
                                .get("parent_thread_id")
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

fn scan_antigravity_sessions(
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
    Ok(())
}

fn scan_opencode_sessions(
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
        "SELECT json_object('id',id,'directory',directory,'modified',datetime(time_updated/1000,'unixepoch')) FROM session",
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

pub(crate) fn system_time_to_rfc3339(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let s = rem % 60;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{s:02}Z")
}

pub(crate) fn days_to_ymd(days_since_1970: u64) -> (u32, u32, u32) {
    let z = days_since_1970 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe + era * 400) as u32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

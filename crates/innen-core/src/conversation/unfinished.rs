//! Discovery of recent unfinished conversations based on structural evidence.
//!
//! A session is classified as a `candidate`, never as a semantic certainty.
//! Structural indicators include active/blocked checkpoints, trailing unanswered
//! user requests, interrupted/cancelled turns, and unresolved tool failures.
//! Sessions with explicit `completed` or `superseded` checkpoints are excluded.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::checkpoint::{latest_checkpoint_for_session, Checkpoint, CheckpointStatus};
use super::resume::{find_project_candidates, system_time_to_rfc3339};
use super::sources::Source;
use super::ReadError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reason {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnfinishedCandidate {
    pub id: String,
    pub source: Source,
    pub project: String,
    pub modified: Option<String>,
    pub classification: String,
    pub confidence: Confidence,
    pub reasons: Vec<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

/// Convert (year, month, day) to days since 1970-01-01 (Gregorian calendar algorithm).
pub(crate) fn ymd_to_days(y: u32, m: u32, d: u32) -> u64 {
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as u64 * 146097 + doe as u64).saturating_sub(719468)
}

/// Parse date/timestamp or duration string into UNIX epoch seconds threshold.
/// Default (None) is the start of yesterday UTC (yesterday + today).
pub fn parse_since(since: Option<&str>) -> Result<u64, ReadError> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days_now = now_secs / 86400;

    let Some(s) = since.map(str::trim).filter(|s| !s.is_empty()) else {
        // Default: start of yesterday (00:00:00 UTC)
        return Ok(days_now.saturating_sub(1) * 86400);
    };

    if s.eq_ignore_ascii_case("today") {
        return Ok(days_now * 86400);
    }
    if s.eq_ignore_ascii_case("yesterday") {
        return Ok(days_now.saturating_sub(1) * 86400);
    }

    // Relative duration e.g. "2d", "48h", "1d"
    if let Some(num_str) = s.strip_suffix('d').or_else(|| s.strip_suffix('D')) {
        if let Ok(days) = num_str.parse::<u64>() {
            return Ok(now_secs.saturating_sub(days * 86400));
        }
    }
    if let Some(num_str) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        if let Ok(hours) = num_str.parse::<u64>() {
            return Ok(now_secs.saturating_sub(hours * 3600));
        }
    }

    // Date "YYYY-MM-DD"
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
            ) {
                if (1..=12).contains(&m) && (1..=31).contains(&d) {
                    return Ok(ymd_to_days(y, m, d) * 86400);
                }
            }
        }
    }

    // RFC3339 datetime e.g. "YYYY-MM-DDTHH:MM:SSZ"
    if s.len() >= 19 && (s.contains('T') || s.contains(' ')) {
        let date_part = &s[..10];
        let time_part = &s[11..19];
        let parts_d: Vec<&str> = date_part.split('-').collect();
        let parts_t: Vec<&str> = time_part.split(':').collect();
        if parts_d.len() == 3 && parts_t.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d), Ok(hr), Ok(mn), Ok(sc)) = (
                parts_d[0].parse::<u32>(),
                parts_d[1].parse::<u32>(),
                parts_d[2].parse::<u32>(),
                parts_t[0].parse::<u64>(),
                parts_t[1].parse::<u64>(),
                parts_t[2].parse::<u64>(),
            ) {
                let base_days = ymd_to_days(y, m, d);
                return Ok(base_days * 86400 + hr * 3600 + mn * 60 + sc);
            }
        }
    }

    Err(ReadError(format!(
        "invalid --since format: '{s}'; expected YYYY-MM-DD, RFC3339 timestamp, duration (e.g. 2d, 48h), today, or yesterday"
    )))
}

/// Convert RFC3339 timestamp string to UNIX epoch seconds.
pub fn rfc3339_to_secs(s: &str) -> Option<u64> {
    if s.len() >= 19 {
        let date_part = &s[..10];
        let time_part = &s[11..19];
        let parts_d: Vec<&str> = date_part.split('-').collect();
        let parts_t: Vec<&str> = time_part.split(':').collect();
        if parts_d.len() == 3 && parts_t.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d), Ok(hr), Ok(mn), Ok(sc)) = (
                parts_d[0].parse::<u32>(),
                parts_d[1].parse::<u32>(),
                parts_d[2].parse::<u32>(),
                parts_t[0].parse::<u64>(),
                parts_t[1].parse::<u64>(),
                parts_t[2].parse::<u64>(),
            ) {
                let days = ymd_to_days(y, m, d);
                return Some(days * 86400 + hr * 3600 + mn * 60 + sc);
            }
        }
    }
    None
}

/// Discovers candidate unfinished conversations across all projects (portfolio discovery).
pub fn find_all_unfinished_candidates(
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let since_secs = parse_since(since)?;
    let base_candidates = super::resume::find_all_candidates(source_filter, source_root)?;

    filter_and_analyze_candidates(base_candidates, since_secs, None)
}

/// Discovers candidate unfinished conversations for a project directory.
pub fn find_unfinished_candidates(
    project_dir: &Path,
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let since_secs = parse_since(since)?;
    let base_candidates = find_project_candidates(project_dir, source_filter, source_root)?;

    filter_and_analyze_candidates(base_candidates, since_secs, Some(project_dir))
}

fn filter_and_analyze_candidates(
    base_candidates: Vec<super::resume::Candidate>,
    since_secs: u64,
    scoped_project: Option<&Path>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let mut unfinished = Vec::new();
    for candidate in base_candidates {
        // 1. Check timestamp against since_secs
        let mod_secs = candidate
            .modified
            .as_deref()
            .and_then(rfc3339_to_secs)
            .unwrap_or_else(|| {
                std::fs::metadata(&candidate.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });

        if mod_secs < since_secs {
            continue;
        }

        let project_str = if let Some(sp) = scoped_project {
            sp.to_string_lossy().to_string()
        } else {
            candidate
                .project
                .clone()
                .unwrap_or_else(|| "unknown".into())
        };

        // 2. Check explicit checkpoint (per-project checkpoints preserved)
        let check_dir: Option<PathBuf> = if let Some(sp) = scoped_project {
            Some(sp.to_path_buf())
        } else if project_str != "unknown" {
            let p = PathBuf::from(&project_str);
            if p.is_dir() {
                Some(p)
            } else {
                None
            }
        } else {
            None
        };

        let latest_cp = if let Some(ref dir) = check_dir {
            latest_checkpoint_for_session(dir, &candidate.id)?
        } else {
            None
        };

        if let Some(ref cp) = latest_cp {
            if matches!(
                cp.status,
                CheckpointStatus::Completed | CheckpointStatus::Superseded
            ) {
                // Explicitly finished or superseded: exclude
                continue;
            }
        }

        // 3. Inspect structural evidence
        let (reasons, confidence) =
            inspect_structural_evidence(&candidate.path, candidate.source, latest_cp.as_ref());

        unfinished.push(UnfinishedCandidate {
            id: candidate.id,
            source: candidate.source,
            project: project_str,
            modified: candidate.modified.or_else(|| {
                std::fs::metadata(&candidate.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(system_time_to_rfc3339)
            }),
            classification: "candidate".to_string(),
            confidence,
            reasons,
            checkpoint: latest_cp,
            path: candidate.path,
            parent_id: candidate.parent_id,
        });
    }

    // Sort by modified descending (newest first)
    unfinished.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(unfinished)
}

/// Inspects structural events in the transcript and returns reason codes and confidence.
fn inspect_structural_evidence(
    transcript_path: &Path,
    source: Source,
    checkpoint: Option<&Checkpoint>,
) -> (Vec<Reason>, Confidence) {
    let mut reasons = Vec::new();

    // Checkpoint evidence
    if let Some(cp) = checkpoint {
        match cp.status {
            CheckpointStatus::Active => {
                reasons.push(Reason {
                    code: "checkpoint_active".into(),
                    detail: format!("explicit active checkpoint updated at {}", cp.updated_at),
                });
            }
            CheckpointStatus::Blocked => {
                reasons.push(Reason {
                    code: "checkpoint_blocked".into(),
                    detail: format!("explicit blocked checkpoint updated at {}", cp.updated_at),
                });
            }
            _ => {}
        }
    }

    // Inspect transcript tail for structural cues
    if let Ok((tail, incomplete_tail)) = bounded_tail_lines(transcript_path) {
        if !tail.is_empty() {
            let mut last_user_idx = None;
            let mut last_assistant_idx = None;
            let mut last_exit_code = None;
            let mut interrupted = false;

            for (i, line) in tail.iter().enumerate() {
                if let Ok(event) = serde_json::from_str::<Value>(line) {
                    match source {
                        Source::Antigravity => {
                            let step_type = event["type"].as_str().unwrap_or("");
                            let status = event["status"].as_str().unwrap_or("");
                            if status == "CANCELLED" || status == "ERROR" {
                                interrupted = true;
                            }
                            if step_type == "USER_INPUT" {
                                last_user_idx = Some(i);
                            } else if step_type == "PLANNER_RESPONSE" {
                                last_assistant_idx = Some(i);
                            }
                        }
                        Source::Codex => {
                            let ty = event["type"].as_str().unwrap_or("");
                            let payload_type = event["payload"]["type"].as_str().unwrap_or("");
                            let status = event["status"].as_str().unwrap_or("");
                            if ty == "event_msg" && matches!(payload_type, "turn_aborted" | "error")
                            {
                                interrupted = true;
                            }
                            if status == "cancelled" {
                                interrupted = true;
                            }
                            if ty == "response_item" && payload_type == "message" {
                                let role = event["payload"]["role"].as_str().unwrap_or("");
                                if role == "user" {
                                    last_user_idx = Some(i);
                                } else if role == "assistant" {
                                    last_assistant_idx = Some(i);
                                }
                            }
                            if ty == "response_item"
                                && matches!(
                                    payload_type,
                                    "function_call_output" | "custom_tool_call_output"
                                )
                            {
                                if let Some(code) = extract_exit_code(&event["payload"]["output"]) {
                                    last_exit_code = Some(code);
                                }
                            }
                        }
                        _ => {
                            if let Some(role) = event.get("role").and_then(Value::as_str) {
                                if role == "user" {
                                    last_user_idx = Some(i);
                                } else if role == "assistant" {
                                    last_assistant_idx = Some(i);
                                }
                            }
                        }
                    }
                }
            }

            if incomplete_tail {
                reasons.push(Reason {
                    code: "bounded_tail_incomplete".into(),
                    detail: "structural inspection used only the bounded transcript tail; older records were not read".into(),
                });
            }

            if interrupted {
                reasons.push(Reason {
                    code: "interrupted_or_cancelled".into(),
                    detail: "transcript records an interrupted, aborted, or cancelled turn".into(),
                });
            }

            if let Some(u_idx) = last_user_idx {
                if last_assistant_idx.is_none() || last_assistant_idx.unwrap() < u_idx {
                    reasons.push(Reason {
                        code: "trailing_user_request_unanswered".into(),
                        detail: "transcript ends with a user request without a following assistant response".into(),
                    });
                }
            }

            if let Some(code) = last_exit_code {
                if code != 0 {
                    reasons.push(Reason {
                        code: "unresolved_tool_failure".into(),
                        detail: format!("last executed tool reported non-zero exit code {code}"),
                    });
                }
            }
        }
    }

    if reasons.is_empty() {
        reasons.push(Reason {
            code: "recent_activity_uncheckpointed".into(),
            detail: "recent activity within time window with no terminal completion record".into(),
        });
    }

    // Determine confidence
    let has_high = reasons.iter().any(|r| {
        matches!(
            r.code.as_str(),
            "checkpoint_active"
                | "checkpoint_blocked"
                | "interrupted_or_cancelled"
                | "trailing_user_request_unanswered"
        )
    });
    let has_medium = reasons.iter().any(|r| r.code == "unresolved_tool_failure");

    let confidence = if has_high {
        Confidence::High
    } else if has_medium {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    (reasons, confidence)
}

fn bounded_tail_lines(path: &Path) -> Result<(VecDeque<String>, bool), std::io::Error> {
    const WINDOW: u64 = 1024 * 1024;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((size - start) as usize);
    file.take(size - start).read_to_end(&mut bytes)?;
    let mut lines = VecDeque::with_capacity(20);
    let mut first = true;
    for line in String::from_utf8_lossy(&bytes).lines() {
        if start > 0 && first {
            first = false;
            continue;
        }
        first = false;
        if lines.len() == 20 {
            lines.pop_front();
        }
        lines.push_back(line.to_owned());
    }
    Ok((lines, start > 0))
}

fn extract_exit_code(output: &Value) -> Option<i32> {
    if let Some(obj) = output.as_object() {
        if let Some(code) = obj.get("exit_code").and_then(Value::as_i64) {
            return i32::try_from(code).ok();
        }
    }
    if let Some(s) = output.as_str() {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("Process exited with code ") {
                if let Ok(code) = rest.trim().parse::<i32>() {
                    return Some(code);
                }
            }
            if let Some(rest) = line.strip_prefix("The command exited with code ") {
                let trimmed = rest.trim().trim_end_matches('.');
                if let Ok(code) = trimmed.parse::<i32>() {
                    return Some(code);
                }
            }
        }
    }
    None
}

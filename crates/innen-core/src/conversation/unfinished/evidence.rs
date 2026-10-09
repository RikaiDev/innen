//! Structural evidence inspection: read the bounded transcript tail and turn
//! what is structurally visible into reason codes and a confidence level. It
//! reports what it saw; it does not judge whether the work was good.

use super::model::{Confidence, Reason};
use crate::conversation::checkpoint::{Checkpoint, CheckpointStatus};
use crate::conversation::sources::Source;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Inspects structural events in the transcript and returns reason codes and confidence.
pub(super) fn inspect_structural_evidence(
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

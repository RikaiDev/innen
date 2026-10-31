//! Structural evidence inspection: read the bounded transcript tail and turn
//! what is structurally visible into reason codes and a confidence level. It
//! reports what it saw; it does not judge whether the work was good.
//!
//! The reason vocabulary is source-agnostic by contract: per-source adapters
//! only *emit* these codes, they never add new ones, so discovery and the CLI
//! classification survive tool format changes.

use super::model::{Confidence, Reason};
use crate::conversation::checkpoint::{Checkpoint, CheckpointStatus};
use crate::conversation::sources::Source;
use crate::conversation::task_state::{TaskItemStatus, TaskState};
use serde_json::Value;
use std::path::Path;

/// Inspects structural events in the transcript and returns reason codes and confidence.
pub(super) fn inspect_structural_evidence(
    transcript_path: &Path,
    source: Source,
    checkpoint: Option<&Checkpoint>,
    task_state: Option<&TaskState>,
    session_id: Option<&str>,
) -> (Vec<Reason>, Confidence) {
    let mut reasons = Vec::new();

    // Checkpoint evidence is source-agnostic: an explicit active or blocked
    // checkpoint is the agent's own statement that work is open.
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

    // Structured task state evidence: the agent wrote its own plan as a tool
    // event, so an item still in progress is a fact rather than a reading.
    if let Some(state) = task_state {
        let in_progress = state
            .items
            .iter()
            .filter(|item| item.status == TaskItemStatus::InProgress)
            .count();
        let pending = state
            .items
            .iter()
            .filter(|item| item.status == TaskItemStatus::Pending)
            .count();
        if in_progress > 0 || pending > 0 {
            reasons.push(Reason {
                code: "task_state_outstanding".into(),
                detail: format!(
                    "structured task state at line {} reports {in_progress} in progress and {pending} pending",
                    state.line
                ),
            });
        }
    }

    // Structured stores report completion natively; each store translates its
    // own format into the shared vocabulary and returns -- a JSONL tail read
    // does not apply to them.
    match source {
        Source::Opencode => {
            opencode_outcome_evidence(transcript_path, session_id, &mut reasons);
            return finish(reasons);
        }
        Source::Mcode => {
            mcode_status_evidence(transcript_path, session_id, &mut reasons);
            return finish(reasons);
        }
        Source::Grok => {
            grok_stream_evidence(transcript_path, &mut reasons);
            return finish(reasons);
        }
        _ => {}
    }

    // Inspect transcript tail for structural cues
    if let Ok((tail, incomplete_tail)) = bounded_tail_lines(transcript_path) {
        if !tail.is_empty() {
            let mut last_user_idx = None;
            let mut last_assistant_idx = None;
            let mut last_exit_code = None;
            let mut exit_idx: Option<usize> = None;
            let mut interrupted_idx: Option<usize> = None;
            let mut turn_start_idx: Option<usize> = None;
            let mut tool_failure_idx: Option<usize> = None;
            let mut tool_success_idx: Option<usize> = None;
            let mut turn_completed_idx: Option<usize> = None;
            let mut last_step_status: Option<(String, usize)> = None;
            let mut last_assistant_stop: Option<(String, usize)> = None;

            for (i, line) in tail.iter().enumerate() {
                if let Ok(event) = serde_json::from_str::<Value>(line) {
                    match source {
                        Source::Copilot => {
                            // Bounded event vocabulary: turn boundaries are
                            // `assistant.turn_start`/`turn_end`; `abort` is a
                            // user-initiated cancellation; `session.error`
                            // records infrastructure noise (auth/provider
                            // labels), not a turn failure, and `session
                            // .shutdown` is an orderly exit rather than a
                            // completion record -- both stay unrecognized.
                            match event["type"].as_str().unwrap_or("") {
                                "user.message" => last_user_idx = Some(i),
                                "assistant.message" => last_assistant_idx = Some(i),
                                "assistant.turn_start" => turn_start_idx = Some(i),
                                "assistant.turn_end" => turn_completed_idx = Some(i),
                                "abort" => interrupted_idx = Some(i),
                                "tool.execution_complete" => {
                                    // `success:false` carries the error code
                                    // (denied/failure/rejected); only a later
                                    // successful tool outcome resolves it.
                                    match event["data"]["success"].as_bool() {
                                        Some(false) => tool_failure_idx = Some(i),
                                        Some(true) => tool_success_idx = Some(i),
                                        None => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                        Source::Antigravity => {
                            let step_type = event["type"].as_str().unwrap_or("");
                            let status = event["status"].as_str().unwrap_or("");
                            if status == "CANCELLED" || status == "ERROR" {
                                interrupted_idx = Some(i);
                            }
                            if !status.is_empty() {
                                last_step_status = Some((status.to_string(), i));
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
                            if ty == "event_msg" && payload_type == "task_complete" {
                                turn_completed_idx = Some(i);
                            }
                            if ty == "event_msg" && matches!(payload_type, "turn_aborted" | "error")
                            {
                                interrupted_idx = Some(i);
                            }
                            if status == "cancelled" {
                                interrupted_idx = Some(i);
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
                                    // The newest exit code is the tail's own;
                                    // a later zero supersedes an earlier failure.
                                    if code != 0 {
                                        last_exit_code = Some(code);
                                        exit_idx = Some(i);
                                    } else {
                                        last_exit_code = None;
                                        exit_idx = None;
                                    }
                                }
                            }
                        }
                        Source::Claude => {
                            // Sidechain subagent records describe a nested
                            // conversation, not the main thread.
                            if event.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                                continue;
                            }
                            if event.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)
                            {
                                interrupted_idx = Some(i);
                            }
                            let message = &event["message"];
                            let role = message["role"].as_str().unwrap_or("");
                            if role == "user" {
                                last_user_idx = Some(i);
                            } else if role == "assistant" {
                                last_assistant_idx = Some(i);
                                if let Some(stop) = message["stop_reason"].as_str() {
                                    last_assistant_stop = Some((stop.to_string(), i));
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

            // Resolve source-native terminal markers with the semantics of
            // the store that recorded them.
            match source {
                Source::Antigravity => {
                    // Per-record status: the newest record's status is the
                    // run's own account of how it stopped. DONE closes the
                    // mid-turn states that preceded it (RUNNING/CANCELLED
                    // belong to earlier steps).
                    if let Some((status, i)) = last_step_status {
                        if status == "DONE" && turn_completed_idx.is_none_or(|c| c < i) {
                            turn_completed_idx = Some(i);
                        }
                    }
                }
                Source::Claude => {
                    // The final main-thread assistant message's stop reason
                    // is the store's account of how the turn ended.
                    if let Some((stop, i)) = last_assistant_stop {
                        if stop == "end_turn" {
                            if turn_completed_idx.is_none_or(|c| c < i) {
                                turn_completed_idx = Some(i);
                            }
                        } else {
                            // tool_use / stop_sequence / max_tokens on the
                            // final assistant message: that turn never
                            // concluded.
                            let latest = interrupted_idx.unwrap_or(i).max(i);
                            interrupted_idx = Some(latest);
                        }
                    }
                }
                _ => {}
            }

            // The tail's current state is whichever native terminal signal is
            // newest; a completion closes the negative signals before it. A
            // turn that started but never concluded is itself a negative
            // signal that the completion must postdate.
            let in_flight = turn_start_idx.filter(|t| turn_completed_idx.is_none_or(|c| *t > c));
            let last_negative = in_flight.map_or(interrupted_idx, |t| {
                Some(interrupted_idx.map_or(t, |n| n.max(t)))
            });
            let completed_wins =
                turn_completed_idx.is_some_and(|c| last_negative.is_none_or(|i| c >= i));

            if completed_wins {
                reasons.push(Reason {
                    code: "explicit_turn_completed".into(),
                    detail: "transcript records the final turn reaching completion".into(),
                });
            } else if last_negative.is_some() {
                reasons.push(Reason {
                    code: "interrupted_or_cancelled".into(),
                    detail: "transcript records an interrupted, aborted, or cancelled turn".into(),
                });
            }

            if let Some(u_idx) = last_user_idx {
                // Trailing is content-based: only an assistant message newer
                // than the user's request answers it. A completed turn or a
                // record status does not -- a turn can end without producing
                // a reply (quota exhaustion, aborted inner call).
                if last_assistant_idx.is_none() || last_assistant_idx.unwrap() < u_idx {
                    reasons.push(Reason {
                        code: "trailing_user_request_unanswered".into(),
                        detail: "transcript ends with a user request without a following assistant response".into(),
                    });
                }
            }

            // Unresolved-failure markers, with closure semantics matched to
            // what each marker attests: a non-zero command exit is the tail's
            // own execution state, superseded by a later completion record;
            // a failed protocol tool outcome stays open until a LATER
            // successful tool outcome -- the turn ending afterwards does not
            // attest that anything was retried or resolved.
            let exit_still_open =
                exit_idx.is_some_and(|e| turn_completed_idx.is_none_or(|c| e > c));
            let tool_failure_open =
                tool_failure_idx.is_some_and(|f| tool_success_idx.is_none_or(|s| f > s));
            if exit_still_open {
                reasons.push(Reason {
                    code: "unresolved_tool_failure".into(),
                    detail: format!(
                        "last executed tool reported non-zero exit code {}",
                        last_exit_code.unwrap_or(1)
                    ),
                });
            } else if tool_failure_open {
                reasons.push(Reason {
                    code: "unresolved_tool_failure".into(),
                    detail: "tool call recorded a failed status".into(),
                });
            }
        }
    }

    finish(reasons)
}

/// Grok writes two streams per session directory: the documented
/// conversation stream (`chat_history.jsonl`) and a runtime event stream
/// (`events.jsonl`). Each stream is judged in its own bounded tail; line
/// indexes are never compared across streams, because they advance
/// independently. The ACP `updates.jsonl` channel is deliberately not
/// consumed: it is not the documented store. Quoted error text inside a
/// `tool_result` in the conversation stream is content, not failure
/// evidence -- only protocol outcomes in the runtime stream count.
mod per_source;
mod tail;

use per_source::{grok_stream_evidence, mcode_status_evidence, opencode_outcome_evidence};
use tail::{bounded_tail_lines, extract_exit_code};

/// Shared classification tail: fall back to recent activity when nothing
/// stronger was observed, then grade confidence by the most specific reason.
fn finish(mut reasons: Vec<Reason>) -> (Vec<Reason>, Confidence) {
    if reasons.is_empty() {
        reasons.push(Reason {
            code: "recent_activity_uncheckpointed".into(),
            detail: "recent activity within time window with no terminal completion record".into(),
        });
    }
    let has_high = reasons.iter().any(|r| {
        matches!(
            r.code.as_str(),
            "checkpoint_active"
                | "checkpoint_blocked"
                | "interrupted_or_cancelled"
                | "trailing_user_request_unanswered"
                | "task_state_outstanding"
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

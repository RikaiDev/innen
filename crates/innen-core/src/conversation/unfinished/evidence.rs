//! Structural evidence inspection: read the bounded transcript tail and turn
//! what is structurally visible into reason codes and a confidence level. It
//! reports what it saw; it does not judge whether the work was good.
//!
//! The reason vocabulary is source-agnostic by contract: per-source adapters
//! only *emit* these codes, they never add new ones, so discovery and the CLI
//! classification survive tool format changes.

use super::model::{Confidence, Reason};
use crate::conversation::checkpoint::{Checkpoint, CheckpointStatus};
use crate::conversation::opencode_db;
use crate::conversation::sources::Source;
use crate::conversation::task_state::{TaskItemStatus, TaskState};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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
                            if ty == "event_msg"
                                && matches!(payload_type, "turn_aborted" | "error")
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
                                if let Some(code) = extract_exit_code(&event["payload"]["output"])
                                {
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
                            if event.get("isApiErrorMessage").and_then(Value::as_bool)
                                == Some(true)
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
            let in_flight = turn_start_idx
                .filter(|t| turn_completed_idx.map_or(true, |c| *t > c));
            let last_negative = in_flight.map_or(interrupted_idx, |t| {
                Some(interrupted_idx.map_or(t, |n| n.max(t)))
            });
            let completed_wins = turn_completed_idx
                .map_or(false, |c| last_negative.map_or(true, |i| c >= i));

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
                exit_idx.map_or(false, |e| turn_completed_idx.map_or(true, |c| e > c));
            let tool_failure_open = tool_failure_idx.map_or(false, |f| {
                tool_success_idx.map_or(true, |s| f > s)
            });
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
fn grok_stream_evidence(transcript_path: &Path, reasons: &mut Vec<Reason>) {
    let mut truncated = false;

    // Conversation stream: content-based trailing question, the same rule
    // as the generic tail read -- a user message newer than the newest
    // assistant record in the tail is unanswered. `reasoning` records are
    // thinking, not answers.
    if let Ok((tail, incomplete)) = bounded_tail_lines(transcript_path) {
        truncated |= incomplete;
        let mut last_user = None;
        let mut last_assistant = None;
        for (i, line) in tail.iter().enumerate() {
            if let Ok(event) = serde_json::from_str::<Value>(line) {
                match event["type"].as_str().unwrap_or("") {
                    "user" => last_user = Some(i),
                    "assistant" => last_assistant = Some(i),
                    _ => {}
                }
            }
        }
        if let Some(u_idx) = last_user {
            if last_assistant.is_none() || last_assistant.unwrap() < u_idx {
                reasons.push(Reason {
                    code: "trailing_user_request_unanswered".into(),
                    detail: "conversation stream ends with a user request without a following assistant response".into(),
                });
            }
        }
    }

    // Runtime stream: protocol outcomes. `turn_ended` carries the turn's
    // own outcome; `tool_completed` carries a per-tool outcome that closes
    // only on a later successful tool outcome -- a turn ending afterwards
    // does not attest that anything was retried; `loop_started` marks an
    // agent turn in flight.
    if let Some(events_path) = transcript_path.parent().map(|dir| dir.join("events.jsonl")) {
        if let Ok((tail, incomplete)) = bounded_tail_lines(&events_path) {
            truncated |= incomplete;
            let mut turn_end: Option<(usize, String)> = None;
            let mut loop_started: Option<usize> = None;
            let mut tool_failed: Option<usize> = None;
            let mut tool_succeeded: Option<usize> = None;
            for (i, line) in tail.iter().enumerate() {
                if let Ok(event) = serde_json::from_str::<Value>(line) {
                    match event["type"].as_str().unwrap_or("") {
                        "turn_ended" => {
                            turn_end = Some((
                                i,
                                event["outcome"].as_str().unwrap_or("").to_string(),
                            ));
                        }
                        "loop_started" => loop_started = Some(i),
                        "tool_completed" => {
                            match event["outcome"].as_str().unwrap_or("") {
                                "error" => tool_failed = Some(i),
                                "success" => tool_succeeded = Some(i),
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }

            // The newest turn-lifecycle record states where the runtime
            // stopped: a loop start with no later turn end is a turn still
            // running; an ended turn carries its own outcome.
            let in_flight = loop_started
                .map_or(false, |l| turn_end.as_ref().map_or(true, |(c, _)| l > *c));
            if in_flight {
                reasons.push(Reason {
                    code: "interrupted_or_cancelled".into(),
                    detail: "agent loop started a turn without a recorded turn end".into(),
                });
            } else if let Some((_, outcome)) = turn_end {
                match outcome.as_str() {
                    "error" => reasons.push(Reason {
                        code: "interrupted_or_cancelled".into(),
                        detail: "last turn ended with an error outcome".into(),
                    }),
                    "success" => reasons.push(Reason {
                        code: "explicit_turn_completed".into(),
                        detail: "last turn ended with a success outcome".into(),
                    }),
                    _ => {}
                }
            }

            if tool_failed.map_or(false, |f| tool_succeeded.map_or(true, |s| f > s)) {
                reasons.push(Reason {
                    code: "unresolved_tool_failure".into(),
                    detail: "tool call recorded an error outcome".into(),
                });
            }
        }
    }

    if truncated {
        reasons.push(Reason {
            code: "bounded_tail_incomplete".into(),
            detail: "structural inspection used only the bounded transcript tail; older records were not read".into(),
        });
    }
}

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

/// OpenCode records a per-turn `outcome` on the newest session message:
/// failed/interrupted turns are open threads; a succeeded turn is the
/// turn-level completion record surfaced as `explicit_turn_completed`.
/// Discovery excludes a session only on an explicit terminal checkpoint.
fn opencode_outcome_evidence(
    transcript_path: &Path,
    session_id: Option<&str>,
    reasons: &mut Vec<Reason>,
) {
    let sql = if let Some(sid) = session_id {
        format!("SELECT json_object('outcome', json_extract(m.data, '$.outcome')) FROM session_message m WHERE m.session_id = '{sid}' ORDER BY m.time_created DESC LIMIT 1")
    } else {
        "SELECT json_object('outcome', json_extract(m.data, '$.outcome')) FROM session_message m ORDER BY m.time_created DESC LIMIT 1".to_string()
    };
    if let Ok(rows) = opencode_db::sqlite(transcript_path, &sql) {
        for row in &rows {
            match row.get("outcome").and_then(Value::as_str) {
                Some("failed") => reasons.push(Reason {
                    code: "unresolved_tool_failure".into(),
                    detail: "session ended with a failed outcome".into(),
                }),
                Some("interrupted") => reasons.push(Reason {
                    code: "interrupted_or_cancelled".into(),
                    detail: "session ended with an interrupted outcome".into(),
                }),
                Some("succeeded") => reasons.push(Reason {
                    code: "explicit_turn_completed".into(),
                    detail: "session ended with a succeeded outcome".into(),
                }),
                _ => {}
            }
        }
    }
}

/// Mcode marks whole-conversation runs in its runtime database. There is no
/// completed status, so aborted and error are the only explicit statements;
/// idle/started sessions stay quiet and classify as recent activity.
fn mcode_status_evidence(
    transcript_path: &Path,
    session_id: Option<&str>,
    reasons: &mut Vec<Reason>,
) {
    let Some(db) = mcode_runtime_db_for(transcript_path) else {
        return;
    };
    let Some(sid) = session_id else {
        return;
    };
    let sql = format!(
        "SELECT json_object('status',status,'error',error_message) FROM local_runtime_sessions WHERE session_id='{sid}' LIMIT 1"
    );
    if let Ok(rows) = opencode_db::sqlite(&db, &sql) {
        for row in &rows {
            match row.get("status").and_then(Value::as_str) {
                Some("aborted") => reasons.push(Reason {
                    code: "interrupted_or_cancelled".into(),
                    detail: "runtime marks the conversation aborted".into(),
                }),
                Some("error") => {
                    let detail = row
                        .get("error")
                        .and_then(Value::as_str)
                        .filter(|e| !e.is_empty())
                        .map(|e| format!("runtime status error: {e}"))
                        .unwrap_or_else(|| "runtime marks the conversation errored".into());
                    reasons.push(Reason {
                        code: "unresolved_tool_failure".into(),
                        detail,
                    });
                }
                _ => {}
            }
        }
    }
}

/// Resolves the mcode runtime-state database from a candidate path: callers
/// pass the transcript, so walk up to the `sessions` directory and read the
/// sibling `sqlite/runtime-state.sqlite`; a candidate already pointing at the
/// database is used directly.
fn mcode_runtime_db_for(path: &Path) -> Option<PathBuf> {
    if path.extension().is_some_and(|e| e == "sqlite") {
        return Some(path.to_path_buf());
    }
    let mut dir = path.parent();
    while let Some(current) = dir {
        if current.file_name().and_then(std::ffi::OsStr::to_str) == Some("sessions") {
            return current
                .parent()
                .map(|base| base.join("sqlite").join("runtime-state.sqlite"));
        }
        dir = current.parent();
    }
    None
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
        if lines.len() == 200 {
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

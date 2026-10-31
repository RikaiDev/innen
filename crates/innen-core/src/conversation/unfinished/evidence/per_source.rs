//! Per-source structural evidence adapters.
//!
//! Each adapter only emits the source-agnostic reason codes; it never adds
//! codes of its own.

use super::super::model::Reason;
use super::tail::bounded_tail_lines;
use crate::conversation::opencode_db;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) fn grok_stream_evidence(transcript_path: &Path, reasons: &mut Vec<Reason>) {
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
                            turn_end =
                                Some((i, event["outcome"].as_str().unwrap_or("").to_string()));
                        }
                        "loop_started" => loop_started = Some(i),
                        "tool_completed" => match event["outcome"].as_str().unwrap_or("") {
                            "error" => tool_failed = Some(i),
                            "success" => tool_succeeded = Some(i),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

            // The newest turn-lifecycle record states where the runtime
            // stopped: a loop start with no later turn end is a turn still
            // running; an ended turn carries its own outcome.
            let in_flight =
                loop_started.is_some_and(|l| turn_end.as_ref().is_none_or(|(c, _)| l > *c));
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

            if tool_failed.is_some_and(|f| tool_succeeded.is_none_or(|s| f > s)) {
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

/// OpenCode records a per-turn `outcome` on the newest session message:
/// failed/interrupted turns are open threads; a succeeded turn is the
/// turn-level completion record surfaced as `explicit_turn_completed`.
/// Discovery excludes a session only on an explicit terminal checkpoint.
pub(super) fn opencode_outcome_evidence(
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
pub(super) fn mcode_status_evidence(
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

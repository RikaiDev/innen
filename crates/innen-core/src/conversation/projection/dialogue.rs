//! Per-source dialogue projection: one native event in, one role/content pair out,
//! with identity and chronology copied from the source rather than invented.

use crate::conversation::{sources::Source, ReadError};
use serde_json::{json, Value};

pub(super) fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("text");
                if matches!(kind, "text" | "input_text" | "output_text")
                    && part.get("thought") != Some(&Value::Bool(true))
                {
                    part.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn dialogue(source: Source, event: Value) -> Result<Option<Value>, ReadError> {
    if source == Source::Vscode {
        let user = event
            .get("message")
            .map(|m| text(m.get("text").unwrap_or(m)))
            .unwrap_or_default();
        let response = event
            .get("response")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["kind"] == "markdownContent")
                    .filter_map(|p| p.pointer("/content/value").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if user.is_empty() && response.is_empty() {
            return Ok(None);
        }
        return Ok(Some(
            json!({"request_id":event["requestId"],"timestamp":event["timestamp"],"messages":[{"role":"user","content":user},{"role":"assistant","content":response}]}),
        ));
    }
    let (role, content) = match source {
        Source::Antigravity => {
            let role = match event["type"].as_str() {
                Some("USER_INPUT") => "user",
                Some("PLANNER_RESPONSE") => "assistant",
                _ => return Ok(None),
            };
            let mut content = text(&event["content"]);
            if role == "user" {
                if let Some(tail) = content.trim_start().strip_prefix("<USER_REQUEST>") {
                    if let Some((request, _)) = tail.split_once("</USER_REQUEST>") {
                        content = request.trim().into();
                    }
                }
            }
            (role, content)
        }
        Source::Codex => {
            // event_msg user_message/agent_message mirrors response_item; don't duplicate.
            if (event["type"] == "event_msg" && event["payload"]["type"] == "item_completed")
                || (event["type"] == "response_item"
                    && event["payload"]["type"] == "command_execution")
            {
                let item = event.pointer("/payload/item").unwrap_or(&event["payload"]);
                if item["type"] != "command_execution" {
                    return Ok(None);
                }
                let content = item
                    .get("aggregated_output")
                    .or_else(|| item.get("output"))
                    .or_else(|| item.get("stdout"))
                    .or_else(|| item.get("stderr"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if content.is_empty() {
                    return Ok(None);
                }
                let prefix = if event["type"] == "response_item" {
                    "/payload"
                } else {
                    "/payload/item"
                };
                let pointer = if item.get("aggregated_output").is_some() {
                    format!("{prefix}/aggregated_output")
                } else if item.get("output").is_some() {
                    format!("{prefix}/output")
                } else if item.get("stdout").is_some() {
                    format!("{prefix}/stdout")
                } else {
                    format!("{prefix}/stderr")
                };
                let mut out = json!({"role":"tool","content":content,
                    "source_pointer":pointer,
                    "source_scalar_sha256":crate::ids::sha256_hex(content.as_bytes())});
                copy_identity(&event, &mut out);
                return Ok(Some(out));
            }
            if event["type"] != "response_item" || event["payload"]["type"] != "message" {
                return Ok(None);
            }
            (
                event["payload"]["role"].as_str().unwrap_or(""),
                text(&event["payload"]["content"]),
            )
        }
        Source::Claude | Source::Cursor => {
            let node = event.get("message").unwrap_or(&event);
            let role = node
                .get("role")
                .or_else(|| event.get("role"))
                .or_else(|| event.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            (role, text(&node["content"]))
        }
        Source::Qwen => {
            let role = match event["type"].as_str() {
                Some("user") => "user",
                Some("assistant") => "assistant",
                _ => return Ok(None),
            };
            (role, text(&event["message"]["parts"]))
        }
        Source::Grok => (
            event
                .get("role")
                .or_else(|| event.get("type"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            text(&event["content"]),
        ),
        Source::Copilot => {
            let role = match event["type"].as_str() {
                Some("user.message") => "user",
                Some("assistant.message") => "assistant",
                _ => return Ok(None),
            };
            (role, text(&event["data"]["content"]))
        }
        Source::Gemini => {
            // Preserve revision/rewind markers in dialogue history rather than
            // silently presenting superseded text as the active resume state.
            if event.get("$rewindTo").is_some() || event.get("$set").is_some() {
                return Ok(Some(event));
            }
            let role = match event["type"].as_str() {
                Some("user") => "user",
                Some("gemini") => "assistant",
                _ => return Ok(None),
            };
            (role, text(&event["content"]))
        }
        Source::Opencode => (
            event["message"]["role"].as_str().unwrap_or(""),
            text(&event["parts"]),
        ),
        Source::Vscode => unreachable!("handled above"),
    };
    if !matches!(role, "user" | "assistant") || content.is_empty() {
        return Ok(None);
    }
    let mut output = json!({"role":role,"content":content});
    copy_identity(&event, &mut output);
    if source == Source::Codex {
        if let Some(phase) = event.pointer("/payload/phase") {
            output["phase"] = phase.clone();
        }
    }
    Ok(Some(output))
}

/// Identity and chronology stay source-native; large model/tool metadata does not.
pub(super) fn copy_identity(event: &Value, output: &mut Value) {
    for key in [
        "id",
        "uuid",
        "step_index",
        "created_at",
        "timestamp",
        "status",
        "type",
        "parentUuid",
        "truncated_fields",
        "ordinal",
    ] {
        if let Some(value) = event.get(key) {
            output[key] = value.clone();
        }
    }
}

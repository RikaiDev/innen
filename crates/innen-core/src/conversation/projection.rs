//! Text projection only; source-native events remain available in events view.
use super::{sources::Source, ReadError};
use serde_json::{json, Value};

fn tool_preview(content: &Value) -> (String, Option<i32>) {
    let body = if let Some(parts) = content.as_array() {
        let mut texts: Vec<&str> = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect();
        if let Some(first) = texts.first() {
            let prologue = first
                .strip_prefix("Script completed\nWall time ")
                .and_then(|s| s.strip_suffix(" seconds\nOutput:\n"));
            if prologue
                .and_then(|s| s.parse::<f64>().ok())
                .is_some_and(|n| n.is_finite() && n >= 0.0)
            {
                texts.remove(0);
            }
        }
        let text = texts.join("\n");
        if text.is_empty() {
            content.to_string()
        } else {
            text
        }
    } else {
        content
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| content.to_string())
    };
    if let Ok(value) = serde_json::from_str::<Value>(&body) {
        if value.get("wall_time_seconds").is_some_and(Value::is_number) {
            if let (Some(code), Some(output)) = (
                value["exit_code"]
                    .as_i64()
                    .and_then(|n| i32::try_from(n).ok()),
                value["output"].as_str(),
            ) {
                return (output.to_owned(), Some(code));
            }
        }
    }
    if let Some((header, output)) = body.split_once("\nOutput:\n") {
        let rows: Vec<_> = header.lines().collect();
        let code = if rows.len() == 4
            && rows[0].starts_with("Chunk ID: ")
            && rows[1].starts_with("Wall time: ")
            && rows[3].starts_with("Original token count: ")
        {
            rows[2]
                .strip_prefix("Process exited with code ")
                .and_then(|s| s.parse::<i32>().ok())
        } else if rows.len() == 2 && rows[1].starts_with("Wall time: ") {
            rows[0]
                .strip_prefix("Exit code: ")
                .and_then(|s| s.parse::<i32>().ok())
        } else {
            None
        };
        if code.is_some() {
            return (output.to_owned(), code);
        }
    }
    (body, None)
}

/// Complete human text and explicitly partial tool navigation in source order.
/// Other dialects retain native events until their mixed-block mapping is
/// implemented, rather than silently losing embedded tool blocks.
pub fn context(source: Source, event: Value) -> Result<Option<Value>, ReadError> {
    if source != Source::Codex {
        return Ok(Some(event));
    }
    if event["type"] == "compacted"
        || (event["type"] == "event_msg"
            && matches!(
                event["payload"]["type"].as_str(),
                Some("error" | "warning" | "turn_aborted")
            ))
    {
        return Ok(Some(event));
    }
    let mut human = dialogue(source, event.clone())?;
    if human.is_none()
        && event["type"] == "response_item"
        && event["payload"]["type"] == "message"
        && matches!(
            event["payload"]["role"].as_str(),
            Some("user" | "assistant")
        )
    {
        let mut empty = json!({"role":event["payload"]["role"],"content":""});
        copy_identity(&event, &mut empty);
        human = Some(empty);
    }
    if let Some(mut human) = human {
        let nontext: Vec<_> = event
            .pointer("/payload/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|p| {
                !matches!(
                    p["type"].as_str(),
                    Some("text" | "input_text" | "output_text")
                )
            })
            .map(|p| p.get("type").cloned().unwrap_or(json!("unknown")))
            .collect();
        if !nontext.is_empty() {
            human["nontext_parts_in_source"] = json!(nontext);
        }
        if let Some(phase) = event.pointer("/payload/phase") {
            human["phase"] = phase.clone();
        }
        Ok(Some(human))
    } else {
        let preview = index(source, event.clone())?;
        if preview.is_none()
            && event["type"] == "response_item"
            && event["payload"]["type"] != "reasoning"
            && !(event["payload"]["type"] == "message"
                && matches!(
                    event["payload"]["role"].as_str(),
                    Some("system" | "developer")
                ))
        {
            return Ok(Some(event));
        }
        Ok(preview)
    }
}

/// Structural navigation only. A preview is explicitly incomplete and never
/// a semantic summary. Other views retain the original source text and fields.
pub fn index(source: Source, event: Value) -> Result<Option<Value>, ReadError> {
    let human = dialogue(source, event.clone())?;
    let mut reported_exit = None;
    let (kind, body, call_id, name) = if let Some(human) = human {
        let body = human
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| human.get("messages").unwrap_or(&human).to_string());
        (
            human["role"].as_str().unwrap_or("exchange").to_owned(),
            body,
            Value::Null,
            Value::Null,
        )
    } else {
        let node = event.get("payload").unwrap_or(&event);
        let kind = node["type"]
            .as_str()
            .or_else(|| event["type"].as_str())
            .unwrap_or("unknown");
        if source == Source::Codex
            && (event["type"] != "response_item"
                || !matches!(
                    kind,
                    "function_call"
                        | "custom_tool_call"
                        | "function_call_output"
                        | "custom_tool_call_output"
                ))
        {
            return Ok(None);
        }
        let content = node
            .get("arguments")
            .or_else(|| node.get("input"))
            .or_else(|| node.get("output"))
            .unwrap_or(node);
        let body = if matches!(kind, "function_call_output" | "custom_tool_call_output") {
            let (body, code) = tool_preview(content);
            reported_exit = code;
            body
        } else {
            content
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| content.to_string())
        };
        (
            kind.to_owned(),
            body,
            node["call_id"].clone(),
            node["name"].clone(),
        )
    };
    let chars = body.chars().count();
    let mut out = json!({"kind":kind,"chars":chars,"preview":body.chars().take(if chars>120 {60} else {120}).collect::<String>(),"preview_truncated":chars>120});
    if chars > 120 {
        out["preview_tail"] = json!(body
            .chars()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>());
    }
    if let Some(code) = reported_exit {
        out["reported_exit_code"] = json!(code);
    }
    if !call_id.is_null() {
        out["call_id"] = call_id;
    }
    if !name.is_null() {
        out["name"] = name;
    }
    if let Some(truncated) = event.get("truncated_fields") {
        out["truncated_fields"] = truncated.clone();
    }
    Ok(Some(out))
}

/// Replace matching opaque call IDs with exact in-page source references.
/// Never infer adjacency; unresolved/colliding identifiers remain explicit.
pub fn link_index(page: &mut super::Page) {
    let mut calls = std::collections::BTreeMap::<String, Vec<usize>>::new();
    for record in &page.records {
        if matches!(
            record.event["kind"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) {
            if let Some(id) = record.event["call_id"].as_str() {
                calls.entry(id.into()).or_default().push(record.line);
            }
        }
    }
    for record in &mut page.records {
        let Some(id) = record.event["call_id"].as_str() else {
            continue;
        };
        if let Some(lines) = calls.get(id).filter(|lines| lines.len() == 1) {
            record.event["call_line"] = json!(lines[0]);
            record
                .event
                .as_object_mut()
                .expect("index object")
                .remove("call_id");
        }
    }
}

fn text(value: &Value) -> String {
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
    Ok(Some(output))
}

fn copy_identity(event: &Value, output: &mut Value) {
    // Identity and chronology stay source-native; large model/tool metadata does not.
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
    ] {
        if let Some(value) = event.get(key) {
            output[key] = value.clone();
        }
    }
}

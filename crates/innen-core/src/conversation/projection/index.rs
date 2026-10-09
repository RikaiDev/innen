//! Structural navigation projection: a bounded preview with explicit truncation
//! and an exact in-page call reference. A preview is never a semantic summary.

use super::dialogue::dialogue;
use crate::conversation::{sources::Source, Page, ReadError};
use serde_json::{json, Value};

/// Recognise the several shapes a harness writes its exit status in, so the
/// projected preview reports the code the tool actually reported.
pub(super) fn tool_preview(content: &Value) -> (String, Option<i32>) {
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
pub fn link_index(page: &mut Page) {
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

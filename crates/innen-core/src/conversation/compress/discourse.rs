//! Discourse unit parsing from conversation records.

use crate::conversation::Record;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct DiscourseUnit {
    pub line: usize,
    pub role: Role,
    pub content: String,
    pub raw_event: Value,
    pub timestamp: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub is_checkpoint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
    pub call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub name: String,
    pub output: String,
    pub success: bool,
    pub call_id: Option<String>,
}

/// Parse records into structured discourse units using conversation structure.
pub fn parse_discourse_units(records: &[Record]) -> Vec<DiscourseUnit> {
    records
        .iter()
        .map(|record| {
            let event = &record.event;
            let role = determine_role(event);
            let content = extract_content(event);
            let timestamp = event
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let tool_calls = extract_tool_calls(event);
            let tool_results = extract_tool_results(event);
            let is_checkpoint = event
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t.contains("checkpoint") || t.contains("task_state"))
                .unwrap_or(false);

            DiscourseUnit {
                line: record.line,
                role,
                content,
                raw_event: event.clone(),
                timestamp,
                tool_calls,
                tool_results,
                is_checkpoint,
            }
        })
        .collect()
}

fn determine_role(event: &Value) -> Role {
    if let Some(r) = event.get("role").and_then(|v| v.as_str()) {
        return match r {
            "user" | "human" => Role::User,
            "assistant" | "model" => Role::Assistant,
            "system" => Role::System,
            "tool" | "function" => Role::Tool,
            _ => Role::Unknown,
        };
    }
    if let Some(ty) = event.get("type").and_then(|v| v.as_str()) {
        return match ty {
            "user_message" | "human_message" => Role::User,
            "assistant_message" | "model_message" => Role::Assistant,
            "tool_result" | "function_result" => Role::Tool,
            "system_message" => Role::System,
            _ => Role::Unknown,
        };
    }
    if event.get("tool_calls").is_some() {
        return Role::Assistant;
    }
    Role::Unknown
}

fn extract_content(event: &Value) -> String {
    event
        .get("content")
        .and_then(|v| v.as_str())
        .or_else(|| event.get("message").and_then(|v| v.as_str()))
        .or_else(|| event.get("text").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string()
}

fn extract_tool_calls(event: &Value) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    if let Some(arr) = event.get("tool_calls").and_then(|v| v.as_array()) {
        for call in arr {
            if let Some(name) = call.get("name").and_then(|v| v.as_str()) {
                calls.push(ToolCall {
                    name: name.to_string(),
                    args: call
                        .get("arguments")
                        .cloned()
                        .unwrap_or(serde_json::json!({})),
                    call_id: call
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }
    }
    calls
}

fn extract_tool_results(event: &Value) -> Vec<ToolResult> {
    let mut results = Vec::new();
    if let Some(arr) = event.get("tool_results").and_then(|v| v.as_array()) {
        for result in arr {
            if let Some(name) = result.get("name").and_then(|v| v.as_str()) {
                results.push(ToolResult {
                    name: name.to_string(),
                    output: result
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    success: result
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    call_id: result
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }
    }
    if event.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
        if let Some(name) = event.get("name").and_then(|v| v.as_str()) {
            results.push(ToolResult {
                name: name.to_string(),
                output: extract_content(event),
                success: event
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                call_id: event
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }
    results
}

/// Extract last user intent from discourse units.
pub fn extract_last_user_intent(units: &[DiscourseUnit]) -> Option<String> {
    for unit in units.iter().rev() {
        if unit.role == Role::User && !unit.content.trim().is_empty() {
            return Some(truncate(&unit.content, 500));
        }
    }
    None
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

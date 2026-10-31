//! Reading the record shapes a transcript carries.
//!
//! The dialects differ in where a call, its arguments and its result live, so
//! the shape knowledge is kept here once instead of in every consumer.

use serde_json::Value;

use super::{Observation, Origin, TaskCounts, TaskItem, TaskItemStatus};

/// Reads every tool observation a raw source record carries.
///
/// The dialects differ in where a call, its arguments and its result live, so
/// the shape knowledge is kept here once instead of in every consumer.
pub(super) fn observations(line: usize, event: &Value) -> Vec<Observation> {
    let mut out = Vec::new();
    let message = event.get("message").unwrap_or(event);
    let role = role_of(message);

    // Harness shape: {"role":"toolResult","toolName":...,"details":{...}}
    if role == Some("toolResult") {
        out.push(from_result(message, line));
    }

    // Harness shape: assistant content parts of type "toolCall".
    if role == Some("assistant") {
        for part in content_parts(message) {
            if matches!(part_type(part), Some("toolCall") | Some("tool_call")) {
                out.push(from_call(part, line));
            }
        }
    }

    // Claude shape: content parts of type "tool_use" and "tool_result".
    for part in content_parts(message) {
        match part_type(part) {
            Some("tool_use") => out.push(from_call(part, line)),
            Some("tool_result") => out.push(from_result(part, line)),
            _ => {}
        }
    }

    // Codex shape: response_item payloads.
    match event
        .pointer("/payload/type")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "function_call" | "custom_tool_call" => {
            out.push(from_call(&event["payload"], line));
        }
        "function_call_output" | "custom_tool_call_output" => {
            let payload = &event["payload"];
            let mut observation = from_result(payload, line);
            observation.failed = observation.failed || payload_failed(payload);
            out.push(observation);
        }
        _ => {}
    }

    // OpenAI shape: a "tool_calls" array on an assistant message.
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            out.push(from_call(call, line));
        }
    }

    // OpenAI shape: a "tool" role message carrying the result.
    if role == Some("tool") {
        out.push(from_result(message, line));
    }

    out
}

fn role_of(value: &Value) -> Option<&str> {
    value.get("role").and_then(Value::as_str)
}

fn part_type(part: &Value) -> Option<&str> {
    part.get("type").and_then(Value::as_str)
}

fn content_parts(message: &Value) -> Vec<&Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| parts.iter().collect())
        .unwrap_or_default()
}

/// Reads a tool call: its name, its identifier and the arguments it carried.
fn from_call(part: &Value, line: usize) -> Observation {
    let tool = part
        .get("name")
        .or_else(|| part.get("toolName"))
        .or_else(|| part.get("tool"))
        .or_else(|| part.pointer("/function/name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let call_id = part
        .get("call_id")
        .or_else(|| part.get("callId"))
        .or_else(|| part.get("toolCallId"))
        .or_else(|| part.get("tool_call_id"))
        .or_else(|| part.get("tool_use_id"))
        .or_else(|| part.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let arguments = parse_arguments(
        part.get("arguments")
            .or_else(|| part.get("input"))
            .or_else(|| part.get("args"))
            .or_else(|| part.get("parameters")),
    );
    let items = arguments.as_ref().and_then(find_items);
    let counts = arguments.as_ref().and_then(find_counts);
    Observation {
        line,
        origin: Origin::Call,
        tool,
        call_id,
        items,
        counts,
        failed: false,
    }
}

/// Reads a tool result: which call it answers, and whether it failed.
fn from_result(part: &Value, line: usize) -> Observation {
    let tool = part
        .get("toolName")
        .or_else(|| part.get("tool_name"))
        .or_else(|| part.get("tool"))
        .or_else(|| part.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let call_id = part
        .get("call_id")
        .or_else(|| part.get("callId"))
        .or_else(|| part.get("toolCallId"))
        .or_else(|| part.get("tool_call_id"))
        .or_else(|| part.get("tool_use_id"))
        .or_else(|| part.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let details = part.get("details");
    let items = details.and_then(find_items).or_else(|| find_items(part));
    let counts = details.and_then(find_counts).or_else(|| find_counts(part));
    Observation {
        line,
        origin: Origin::Result,
        tool,
        call_id,
        items,
        counts,
        failed: result_failed(part),
    }
}

/// True when the result itself reports that the tool failed or was cancelled.
fn result_failed(part: &Value) -> bool {
    for key in ["isError", "is_error", "failed", "error"] {
        if part.get(key).and_then(Value::as_bool) == Some(true) {
            return true;
        }
    }
    if part.get("success").and_then(Value::as_bool) == Some(false) {
        return true;
    }
    if let Some(status) = part.get("status").and_then(Value::as_str) {
        let key: String = status
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if matches!(
            key.as_str(),
            "failed" | "error" | "cancelled" | "canceled" | "aborted"
        ) {
            return true;
        }
    }
    false
}

/// True when a Codex tool output reports a non-zero exit status.
fn payload_failed(payload: &Value) -> bool {
    if payload.get("is_error").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let Some(output) = payload.get("output") else {
        return false;
    };
    let parsed = match output {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        other => Some(other.clone()),
    };
    parsed
        .as_ref()
        .and_then(|value| value.get("exit_code"))
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0)
}

/// Tool arguments arrive as an object or as a JSON string, depending on the
/// harness. Both are read as the same structure.
fn parse_arguments(raw: Option<&Value>) -> Option<Value> {
    match raw? {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        Value::Null => None,
        other => Some(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// item shapes
// ---------------------------------------------------------------------------

/// Keys that hold a list of planned items.
const ITEM_ARRAY_KEYS: &[&str] = &[
    "todos",
    "todoList",
    "items",
    "plan",
    "planItems",
    "steps",
    "tasks",
];

/// Keys that hold the text of one planned item.
const ITEM_TEXT_KEYS: &[&str] = &[
    "content",
    "text",
    "title",
    "task",
    "subject",
    "description",
    "name",
];

/// Keys that hold the status of one planned item.
const ITEM_STATUS_KEYS: &[&str] = &["status", "state"];

/// Tool names that carry a task list. Compared after lowercasing and removing
/// separators, so `TodoWrite`, `todo_write` and `todowrite` all match.
const TASK_STATE_TOOLS: &[&str] = &[
    "todowrite",
    "writetodos",
    "createtodos",
    "updatetodos",
    "settodo",
    "settodos",
    "updateplan",
    "updateplanlist",
    "updatetasklist",
    "createtasklist",
    "tasklist",
    "todo",
    "plan",
];

/// True when a tool name is one that carries a task list.
pub(super) fn is_task_state_tool(name: &str) -> bool {
    let key: String = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    TASK_STATE_TOOLS.contains(&key.as_str())
}

/// Finds the list of planned items inside a tool payload.
fn find_items(container: &Value) -> Option<Vec<TaskItem>> {
    let object = container.as_object()?;
    for key in ITEM_ARRAY_KEYS {
        let Some(entries) = object.get(*key).and_then(Value::as_array) else {
            continue;
        };
        let items: Vec<TaskItem> = entries.iter().filter_map(parse_item).collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    None
}

/// Reads the counts a tool reported next to its items.
fn find_counts(container: &Value) -> Option<TaskCounts> {
    let object = container.as_object()?;
    let read = |keys: &[&str]| -> Option<usize> {
        keys.iter()
            .find_map(|key| object.get(*key))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
    };
    let counts = TaskCounts {
        total: read(&["total"]),
        active: read(&["active"]),
        completed: read(&["completed"]),
        cancelled: read(&["cancelled", "canceled"]),
    };
    if counts.total.is_none()
        && counts.active.is_none()
        && counts.completed.is_none()
        && counts.cancelled.is_none()
    {
        None
    } else {
        Some(counts)
    }
}

/// Reads one planned item.
fn parse_item(entry: &Value) -> Option<TaskItem> {
    let object = entry.as_object()?;
    let content = ITEM_TEXT_KEYS
        .iter()
        .find_map(|key| object.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_string();
    let raw_status = ITEM_STATUS_KEYS
        .iter()
        .find_map(|key| object.get(*key))
        .and_then(Value::as_str)
        .map(str::to_string);
    let status = match raw_status.as_deref() {
        None => TaskItemStatus::Pending,
        Some(raw) => TaskItemStatus::parse(raw).unwrap_or(TaskItemStatus::Unknown),
    };
    Some(TaskItem {
        content,
        status,
        raw_status,
    })
}

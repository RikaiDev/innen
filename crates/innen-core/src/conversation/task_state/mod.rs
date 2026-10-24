//! Structured task-state projection.
//!
//! An agent that tracks its own plan already writes that plan as structured
//! tool events, not as prose. This module recovers the state from a
//! transcript so that:
//!
//! - a continuation never has to ask the model to restate its plan, and
//! - a plan written before a compaction is still visible after it.
//!
//! Two rules keep the projection honest:
//!
//! 1. State comes only from a task-state tool event. Nothing here infers
//!    progress from the wording of a message, and no item is invented.
//! 2. A write counts only once a successful tool result confirms it. A write
//!    whose result reported failure or cancellation is discarded, and an
//!    unconfirmed write is reported as such rather than as fact.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Status of one planned item, as the writing tool reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskItemStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
    /// The tool wrote a status string this reader does not recognise. The
    /// original string is preserved in [`TaskItem::raw_status`] so a caller
    /// never has to guess what the tool actually said.
    Unknown,
}

impl TaskItemStatus {
    /// The stable name of this status, as reported to callers.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// Normalises a status string written by a task-state tool.
    ///
    /// Returns `None` for a status this reader cannot map, so an unmapped
    /// status becomes [`TaskItemStatus::Unknown`] instead of a silent
    /// `Pending`.
    pub fn parse(raw: &str) -> Option<Self> {
        let key: String = raw
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match key.as_str() {
            "pending" | "todo" | "notstarted" | "open" | "new" | "waiting" | "queued"
            | "planned" | "backlog" => Some(Self::Pending),
            "inprogress" | "active" | "doing" | "running" | "started" | "working" | "ongoing"
            | "current" => Some(Self::InProgress),
            "completed" | "complete" | "done" | "finished" | "resolved" | "closed"
            | "succeeded" => Some(Self::Completed),
            "cancelled" | "canceled" | "aborted" | "rejected" | "dropped" | "skipped"
            | "obsolete" | "superseded" | "wontfix" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// One planned item as the writing tool recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    /// The item text the tool wrote.
    pub content: String,
    pub status: TaskItemStatus,
    /// The status string exactly as the tool wrote it, when it wrote one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_status: Option<String>,
}

/// Counts the writing tool reported next to the items.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<usize>,
}

/// The task state recovered from a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskState {
    /// The tool that wrote the state, for example `todowrite`.
    pub tool: String,
    /// The items, in the order the tool wrote them.
    pub items: Vec<TaskItem>,
    /// One-based source line of the write that produced this state.
    pub line: usize,
    /// False when the write never produced a successful tool result, so it may
    /// never have taken effect.
    pub confirmed: bool,
    /// Counts the tool reported next to the items, when it reported any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<TaskCounts>,
}

impl TaskState {
    /// The item the agent was working on when the transcript ended.
    pub fn current(&self) -> Option<&TaskItem> {
        self.items
            .iter()
            .find(|item| item.status == TaskItemStatus::InProgress)
    }

    /// The first item still waiting to be started.
    pub fn next_pending(&self) -> Option<&TaskItem> {
        self.items
            .iter()
            .find(|item| item.status == TaskItemStatus::Pending)
    }

    /// Items that are neither completed nor cancelled.
    pub fn outstanding(&self) -> usize {
        self.items
            .iter()
            .filter(|item| {
                !matches!(
                    item.status,
                    TaskItemStatus::Completed | TaskItemStatus::Cancelled
                )
            })
            .count()
    }
}

/// The recovered state and what the reader could not read.
#[derive(Debug, Clone, Default)]
pub struct TaskStateExtraction {
    /// The authoritative task state, when the transcript carried one.
    pub state: Option<TaskState>,
    /// Source lines that were not valid JSON. A projection cannot recover
    /// state across a line it could not read, so the count travels with the
    /// result instead of disappearing.
    pub unparsed_lines: usize,
}

// ---------------------------------------------------------------------------
// extraction
// ---------------------------------------------------------------------------

/// Recovers the task state from a JSONL transcript.
///
/// The whole file is scanned, not only its tail: the authoritative write may
/// sit far behind the end of the transcript, which is exactly the case a
/// bounded tail would lose.
pub fn extract_file(path: &Path) -> TaskStateExtraction {
    // Only JSONL transcripts carry tool events this reader understands. A
    // database or a whole-document export is left to the source adapter.
    if path.extension().is_none_or(|ext| ext != "jsonl") {
        return TaskStateExtraction::default();
    }
    let Ok(file) = File::open(path) else {
        return TaskStateExtraction::default();
    };
    let reader = BufReader::new(file);
    let mut extractor = Extractor::default();
    for (index, line) in reader.lines().enumerate() {
        let Ok(line) = line else {
            extractor.unparsed_lines += 1;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(event) => extractor.observe(index + 1, &event),
            Err(_) => extractor.unparsed_lines += 1,
        }
    }
    extractor.finish()
}

/// Recovers the task state from already-parsed events, in source order.
pub fn extract_events<I>(events: I) -> TaskStateExtraction
where
    I: IntoIterator<Item = Value>,
{
    let mut reader = TaskStateReader::new();
    for (index, event) in events.into_iter().enumerate() {
        reader.observe(index + 1, &event);
    }
    reader.finish()
}

/// Reads source events in order and keeps the task state they carry.
///
/// A caller that already parses the transcript can feed the same events here
/// instead of reading the file a second time.
#[derive(Debug, Default)]
pub struct TaskStateReader {
    extractor: Extractor,
}

impl TaskStateReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one source event. `line` is its one-based source line.
    pub fn observe(&mut self, line: usize, event: &Value) {
        self.extractor.observe(line, event);
    }

    /// The state the observed events carry.
    pub fn finish(self) -> TaskStateExtraction {
        self.extractor.finish()
    }
}

/// One task-state write seen in the transcript.
#[derive(Debug, Clone)]
struct Write {
    line: usize,
    tool: String,
    items: Vec<TaskItem>,
    counts: Option<TaskCounts>,
}

#[derive(Debug, Default)]
struct Extractor {
    /// The last write a successful tool result confirmed.
    confirmed: Option<Write>,
    /// Writes that are still waiting for a tool result, keyed by call id.
    pending: BTreeMap<String, Write>,
    /// Writes that have no call id to pair a result with.
    unpaired: Vec<Write>,
    unparsed_lines: usize,
}

impl Extractor {
    fn observe(&mut self, line: usize, event: &Value) {
        for observation in observations(line, event) {
            match observation.origin {
                Origin::Call => self.observe_call(observation),
                Origin::Result => self.observe_result(observation),
            }
        }
    }

    fn observe_call(&mut self, observation: Observation) {
        let Some(items) = observation.items else {
            return;
        };
        if !is_task_state_tool(&observation.tool) {
            return;
        }
        let write = Write {
            line: observation.line,
            tool: observation.tool.clone(),
            items,
            counts: observation.counts,
        };
        match observation.call_id.clone() {
            Some(id) => {
                self.pending.insert(id, write);
            }
            None => {
                self.unpaired.push(write);
            }
        }
    }

    fn observe_result(&mut self, observation: Observation) {
        // A result that carries the items is itself the authoritative write.
        if let Some(items) = observation.items {
            if observation.failed {
                // The tool refused the write. Earlier state stands.
                return;
            }
            let tool = if observation.tool.is_empty() {
                self.tool_for(&observation.call_id)
            } else {
                observation.tool.clone()
            };
            if !is_task_state_tool(&tool) {
                return;
            }
            let write = Write {
                line: observation.line,
                tool,
                items,
                counts: observation.counts,
            };
            if let Some(id) = &observation.call_id {
                self.pending.remove(id);
            }
            self.confirmed = Some(write);
            return;
        }
        // A result without items confirms or rejects the matching call.
        let Some(id) = observation.call_id.clone() else {
            return;
        };
        let Some(write) = self.pending.remove(&id) else {
            return;
        };
        if observation.failed {
            return;
        }
        self.confirmed = Some(write);
    }

    fn tool_for(&self, call_id: &Option<String>) -> String {
        call_id
            .as_ref()
            .and_then(|id| self.pending.get(id))
            .map(|write| write.tool.clone())
            .unwrap_or_default()
    }

    fn finish(mut self) -> TaskStateExtraction {
        if let Some(write) = self.confirmed.take() {
            return TaskStateExtraction {
                state: Some(write.into_state(true)),
                unparsed_lines: self.unparsed_lines,
            };
        }
        // No write was confirmed. Report the most recent unconfirmed write so
        // a caller can see that a plan exists but may never have taken effect.
        let mut candidates: Vec<Write> = self.pending.into_values().collect();
        candidates.extend(self.unpaired);
        let newest = candidates.into_iter().max_by_key(|write| write.line);
        TaskStateExtraction {
            state: newest.map(|write| write.into_state(false)),
            unparsed_lines: self.unparsed_lines,
        }
    }
}

impl Write {
    fn into_state(self, confirmed: bool) -> TaskState {
        TaskState {
            tool: self.tool,
            items: self.items,
            line: self.line,
            confirmed,
            counts: self.counts,
        }
    }
}

// ---------------------------------------------------------------------------
// record shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Call,
    Result,
}

#[derive(Debug, Clone)]
struct Observation {
    line: usize,
    origin: Origin,
    tool: String,
    call_id: Option<String>,
    items: Option<Vec<TaskItem>>,
    counts: Option<TaskCounts>,
    failed: bool,
}

/// Reads every tool observation a raw source record carries.
///
/// The dialects differ in where a call, its arguments and its result live, so
/// the shape knowledge is kept here once instead of in every consumer.
fn observations(line: usize, event: &Value) -> Vec<Observation> {
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
fn is_task_state_tool(name: &str) -> bool {
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

#[cfg(test)]
mod tests;

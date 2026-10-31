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

mod parse;

use parse::{is_task_state_tool, observations};

#[cfg(test)]
mod tests;

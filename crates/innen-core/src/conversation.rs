//! Read-only conversation reader. Source adapters resolve native session IDs.
//! Dialogue is a projection, not a summary: preserve corrections and chronology.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub mod attachments;
pub mod checkpoint;
pub mod compact;
pub mod deltas;
pub mod grammar;
pub mod pickup;
mod prefixes;
mod projection;
pub mod resume;
mod sources;
pub mod unfinished;
pub use checkpoint::{Checkpoint, CheckpointStatus};
pub use pickup::{
    resolve_all_projects_pickup, resolve_pickup, EvidencePointers, PickedUpSession, PickupTarget,
};
pub use resume::{find_all_candidates, find_project_candidates, Candidate, ResumeTarget};
pub use sources::{Located, Source};
pub use unfinished::{
    find_all_unfinished_candidates, find_unfinished_candidates, Confidence, Reason,
    UnfinishedCandidate,
};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ReadError(pub(crate) String);

#[derive(Debug, Serialize)]
pub struct Page {
    pub session_id: String,
    pub source: Source,
    pub source_path: PathBuf,
    pub view: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub records: Vec<Record>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Record {
    /// One-based source record position (JSONL line, document item, or database message).
    pub line: usize,
    pub event: Value,
}

#[derive(Debug, Serialize)]
pub struct Brief {
    pub session_id: String,
    pub source: Source,
    pub source_path: PathBuf,
    pub project: Option<String>,
    pub supported: bool,
    pub latest_user: Option<Record>,
    pub latest_assistant: Option<Record>,
    pub latest_agent_messages: Vec<Record>,
    pub unresolved: Vec<EvidencePointer>,
    pub checkpoint: Option<checkpoint::Checkpoint>,
    pub omitted_records: usize,
    pub coverage_incomplete: bool,
    pub warnings: Vec<String>,
    pub expansion: String,
}

#[derive(Debug, Serialize)]
pub struct EvidencePointer {
    pub line: usize,
    pub marker: String,
    pub basis: String,
    pub preview: String,
}

/// Bounded structural continuation brief. It keeps literal source records and
/// pointers; it never invents an objective from queue or tool output text.
pub fn brief(source_root: Option<&Path>, source: &str, id: &str) -> Result<Brief, ReadError> {
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    let mut out = Brief {
        session_id: id.clone(),
        source: located.source,
        source_path: located.path.clone(),
        project: None,
        supported: located.source == Source::Codex
            && located.path.extension().is_some_and(|ext| ext == "jsonl"),
        latest_user: None,
        latest_assistant: None,
        latest_agent_messages: Vec::new(),
        unresolved: Vec::new(),
        checkpoint: None,
        omitted_records: 0,
        coverage_incomplete: false,
        warnings: Vec::new(),
        expansion: format!(
            "innen conversation {id} --source {}{} --view events --lines <source-lines>",
            located.source.as_str(),
            source_root
                .map(|root| format!(
                    " --source-root {}",
                    shell_quote(&root.display().to_string())
                ))
                .unwrap_or_default()
        ),
    };
    if out.supported {
        let file = std::fs::File::open(&located.path).map_err(|e| io_error(&located.path, e))?;
        const LINE_BUDGET: usize = 512 * 1024;
        let mut reader = BufReader::new(file);
        let mut index = 0usize;
        let mut last_oversized_line = None;
        while let Some(line) =
            read_capped_line(&mut reader, LINE_BUDGET).map_err(|e| io_error(&located.path, e))?
        {
            index += 1;
            if line.oversized {
                out.omitted_records += 1;
                out.coverage_incomplete = true;
                last_oversized_line = Some(index);
                out.warnings.push(format!("omitted oversized source line {index}; latest selected records may be incomplete; expand it with --lines"));
                continue;
            }
            // Streaming the selected JSONL gives us the actual physical source line.
            let source_line = index;
            let line_text = String::from_utf8(line.bytes).map_err(|_| {
                ReadError(format!(
                    "invalid UTF-8 in transcript at {}:{}",
                    located.path.display(),
                    index
                ))
            })?;
            if line_text.trim().is_empty() {
                continue;
            }
            let event = serde_json::from_str::<Value>(&line_text).map_err(|error| {
                ReadError(format!(
                    "invalid transcript JSON at {}:{}: {error}",
                    located.path.display(),
                    index
                ))
            })?;
            if event["type"] == "session_meta" {
                out.project = event["payload"]["cwd"].as_str().map(str::to_owned);
            }
            let record = Record {
                line: source_line,
                event: bounded_event(event.clone(), source_line),
            };
            let user = event["payload"]["type"] == "message" && event["payload"]["role"] == "user";
            let assistant =
                event["payload"]["type"] == "message" && event["payload"]["role"] == "assistant";
            let text = event_text(&event);
            let injected = is_injected_envelope(&text);
            if user && !injected && newer(&record, out.latest_user.as_ref()) {
                out.latest_user = Some(record.clone());
            }
            let author = event["payload"]["author"]
                .as_str()
                .or_else(|| event.get("author").and_then(Value::as_str));
            if assistant
                && author.is_none()
                && event["payload"]["type"] != "agent_message"
                && newer(&record, out.latest_assistant.as_ref())
            {
                out.latest_assistant = Some(record.clone());
            }
            if let Some(author) = author {
                if !author.is_empty() {
                    if let Some(existing) = out.latest_agent_messages.iter_mut().find(|r| {
                        r.event["payload"]["author"]
                            .as_str()
                            .or_else(|| r.event.get("author").and_then(Value::as_str))
                            == Some(author)
                    }) {
                        if newer(&record, Some(existing)) {
                            *existing = record.clone();
                        }
                    } else if out.latest_agent_messages.len() < 64 {
                        out.latest_agent_messages.push(record.clone());
                    } else {
                        out.omitted_records += 1;
                    }
                }
            }
        }
        if let Some(line) = last_oversized_line {
            if out
                .latest_user
                .as_ref()
                .is_some_and(|record| record.line < line)
            {
                out.latest_user = None;
            }
            if out
                .latest_assistant
                .as_ref()
                .is_some_and(|record| record.line < line)
            {
                out.latest_assistant = None;
            }
            out.latest_agent_messages
                .retain(|record| record.line >= line);
        }
        collect_latest_signals(&mut out);
    } else {
        out.omitted_records = 1;
        out.warnings.push("brief latest-record extraction is unavailable for this source adapter; use explicit --view context/events".into());
    }
    out.latest_agent_messages.sort_by_key(|r| r.line);
    Ok(out)
}

fn newer(candidate: &Record, current: Option<&Record>) -> bool {
    let stamp = |r: &Record| {
        r.event
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    current.is_none_or(|r| (stamp(candidate), candidate.line) > (stamp(r), r.line))
}

fn collect_latest_signals(brief: &mut Brief) {
    brief.unresolved.clear();
    let mut selected = Vec::new();
    if let Some(record) = brief.latest_user.clone() {
        selected.push(record);
    }
    if let Some(record) = brief.latest_assistant.clone() {
        selected.push(record);
    }
    selected.extend(brief.latest_agent_messages.clone());
    for record in selected {
        let text = event_text(&record.event);
        let lower = text.to_ascii_lowercase();
        for marker in ["queued", "pending", "blocked", "failed"] {
            if lower.contains(marker) {
                brief.unresolved.push(EvidencePointer {
                    line: record.line,
                    marker: marker.into(),
                    basis: "lexical_signal_in_latest_selected_record".into(),
                    preview: text.chars().take(240).collect(),
                });
                break;
            }
        }
    }
}

struct CappedLine {
    bytes: Vec<u8>,
    oversized: bool,
}

fn read_capped_line(
    reader: &mut BufReader<std::fs::File>,
    budget: usize,
) -> std::io::Result<Option<CappedLine>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            return Ok(Some(CappedLine { bytes, oversized }));
        }
        if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
            let take = pos + 1;
            if !oversized && bytes.len() + pos <= budget {
                bytes.extend_from_slice(&available[..pos]);
            } else {
                oversized = true;
            }
            reader.consume(take);
            return Ok(Some(CappedLine { bytes, oversized }));
        }
        if !oversized {
            if bytes.len() + available.len() <= budget {
                bytes.extend_from_slice(available);
            } else {
                oversized = true;
                bytes.clear();
            }
        }
        let take = available.len();
        reader.consume(take);
    }
}

fn is_injected_envelope(text: &str) -> bool {
    let trimmed = text.trim();
    [
        ("<environment_context>", "</environment_context>"),
        ("<user_information>", "</user_information>"),
    ]
    .iter()
    .any(|(open, close)| {
        trimmed.starts_with(open)
            && trimmed.ends_with(close)
            && trimmed.len() > open.len() + close.len()
    }) || {
        let trimmed = text.trim();
        trimmed.starts_with("<INSTRUCTIONS>\n# AGENTS.md") && trimmed.ends_with("</INSTRUCTIONS>")
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn event_text(event: &Value) -> String {
    let content = event.pointer("/payload/content").unwrap_or(event);
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn bounded_event(mut event: Value, line: usize) -> Value {
    if event.to_string().len() <= 16 * 1024 {
        return event;
    }
    if let Some(payload) = event.get_mut("payload").and_then(Value::as_object_mut) {
        payload.insert(
            "content".into(),
            Value::String(format!(
                "[brief omitted oversized content; expand source line {line}]"
            )),
        );
        payload.insert(
            "brief_truncated_fields".into(),
            Value::Array(vec![Value::String("content".into())]),
        );
    }
    if event.to_string().len() > 16 * 1024 {
        return serde_json::json!({"brief_truncated": true, "source_line": line});
    }
    event
}

/// Offset counts physical rows in both views. Read one matching event ahead so
/// next_offset signals actual remaining content, not merely reaching the limit.
/// Invalid source data fails visibly with line provenance, never an empty success.
pub fn read(
    source_root: Option<&Path>,
    source: &str,
    id: &str,
    view: &str,
    offset: usize,
    limit: usize,
) -> Result<Page, ReadError> {
    if !matches!(view, "dialogue" | "events" | "index" | "context") || !(1..=100).contains(&limit) {
        return Err(ReadError(
            "expected view dialogue|events|index|context and limit 1..100".into(),
        ));
    }
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    let mut page = read_located(located, id, view, offset, limit, None)?;
    if view == "index" || (view == "context" && page.source == Source::Codex) {
        projection::link_index(&mut page);
    }
    Ok(page)
}

/// Read explicitly chosen one-based source rows as a single events page. The
/// selection is caller-owned, sorted in source order, and never relevance-ranked.
pub fn read_lines(
    source_root: Option<&Path>,
    source: &str,
    id: &str,
    lines: &[usize],
) -> Result<Page, ReadError> {
    let selected: BTreeSet<_> = lines.iter().copied().collect();
    if selected.is_empty() || selected.len() > 100 || selected.contains(&0) {
        return Err(ReadError(
            "expected 1..100 distinct positive source lines".into(),
        ));
    }
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    let page = read_located(
        located,
        id,
        "events",
        selected.first().unwrap() - 1,
        selected.len(),
        Some(&selected),
    )?;
    for line in &selected {
        if !page.records.iter().any(|r| r.line == *line) {
            return Err(ReadError(format!(
                "requested source line {line} is absent or blank"
            )));
        }
    }
    Ok(page)
}

pub fn format_page(
    page: Page,
    compact: bool,
    deltas: bool,
    attachment_refs: bool,
    attachment: Option<&str>,
    expect_sha256: Option<&str>,
    offset: usize,
) -> Result<Value, ReadError> {
    let ordinary = (compact || attachment_refs)
        .then(|| serde_json::to_value(&page).expect("conversation page serializes"));
    let page = if let Some(pointer) = attachment {
        let hash =
            expect_sha256.ok_or_else(|| ReadError("attachment requires expect_sha256".into()))?;
        attachments::resolve(&page, offset, pointer, hash)?
    } else if attachment_refs {
        attachments::externalize(page, compact)?
    } else if compact {
        compact::encode(&page)
    } else {
        serde_json::to_value(&page).expect("conversation page serializes")
    };
    let guided = if deltas {
        let base = compact::with_guide(page.clone());
        let delta = compact::with_guide(deltas::encode(page));
        if delta.to_string().len() < base.to_string().len() {
            delta
        } else {
            base
        }
    } else {
        compact::with_guide(page)
    };
    let page = match ordinary {
        Some(ordinary) if guided.to_string().len() > ordinary.to_string().len() => ordinary,
        _ => guided,
    };
    Ok(page)
}

fn read_located(
    located: Located,
    id: String,
    view: &str,
    offset: usize,
    limit: usize,
    selected: Option<&BTreeSet<usize>>,
) -> Result<Page, ReadError> {
    let path = located.path.clone();
    let mut page = Page {
        session_id: id.clone(),
        source: located.source,
        source_path: path.clone(),
        view: view.into(),
        offset,
        next_offset: None,
        records: Vec::new(),
        warnings: Vec::new(),
    };
    if located.source == Source::Cursor {
        page.warnings.push("Cursor exported transcripts may omit tool outputs; events cannot restore data absent from the export".into());
    }
    if located.source == Source::Opencode {
        let mut cursor = offset;
        loop {
            let events = sources::load_database(&located, &id, cursor)?;
            if events.is_empty() {
                return Ok(page);
            }
            let count = events.len();
            for (index, event) in events.into_iter().enumerate() {
                if accept(&mut page, event, cursor + index, limit, selected)? {
                    return Ok(page);
                }
            }
            cursor += count;
        }
    }
    if path.extension().is_some_and(|s| s == "json") {
        let events = sources::load_document(&located)?;
        for (index, event) in events.into_iter().enumerate().skip(offset) {
            if accept(&mut page, event, index, limit, selected)? {
                break;
            }
        }
        return Ok(page);
    }
    let file = File::open(&path).map_err(|e| io_error(&path, e))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| io_error(&path, e))?;
        if selected.is_some_and(|s| index + 1 > *s.last().unwrap()) {
            break;
        }
        if index < offset
            || line.trim().is_empty()
            || selected.is_some_and(|s| !s.contains(&(index + 1)))
        {
            continue;
        }
        let event: Value = serde_json::from_str(&line).map_err(|e| {
            ReadError(format!(
                "invalid transcript JSON at {}:{}: {e}",
                path.display(),
                index + 1
            ))
        })?;
        if accept(&mut page, event, index, limit, selected)? {
            break;
        }
    }
    Ok(page)
}

fn accept(
    page: &mut Page,
    event: Value,
    index: usize,
    limit: usize,
    selected: Option<&BTreeSet<usize>>,
) -> Result<bool, ReadError> {
    if let Some(selected) = selected {
        if !selected.contains(&(index + 1)) {
            return Ok(index + 1 > *selected.last().unwrap());
        }
    }
    if !event.is_object() {
        return Err(ReadError(format!(
            "unsupported transcript event at {}:{}: expected object",
            page.source_path.display(),
            index + 1
        )));
    }
    let event = if page.view == "events" {
        Some(event)
    } else if page.view == "context" {
        projection::context(page.source, event)?
    } else if page.view == "index" {
        projection::index(page.source, event)?
    } else {
        projection::dialogue(page.source, event)?
    };
    let Some(event) = event else { return Ok(false) };
    if page.records.len() == limit {
        page.next_offset = Some(index);
        return Ok(true);
    }
    if event
        .get("truncated_fields")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        page.warnings.push(format!("source event at record {} contains truncated fields; this reader cannot restore omitted source text", index + 1));
    }
    page.records.push(Record {
        line: index + 1,
        event,
    });
    Ok(selected.is_some_and(|s| page.records.len() == s.len()))
}

fn io_error(path: &Path, error: std::io::Error) -> ReadError {
    ReadError(format!("read {}: {error}", path.display()))
}

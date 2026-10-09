use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::bounded::{
    bounded_event, event_text, is_injected_envelope, read_capped_line, shell_quote,
};
use super::read::io_error;
use super::{checkpoint, sources, ReadError, Source};

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

//! Phase 1: map a page's records to the tool calls and tool outputs they carry.
//!
//! One pass over `page.records` in every dialect the reader can emit: context
//! events, dialogue turns, the `tool_calls` array, Claude's `message.content`
//! parts, and raw Codex payloads. Later phases only see this neutral shape.

use std::collections::BTreeMap;

use serde_json::Value;

use super::classify::{extract_tool_action, ToolAction};

pub(super) struct CallMeta {
    pub record_idx: usize,
    pub source_line: usize,
    pub call_id: Option<String>,
    pub action: ToolAction,
}

pub(super) struct OutputMeta {
    pub record_idx: usize,
    pub source_line: usize,
    pub call_id: Option<String>,
    pub call_line: Option<usize>,
    pub is_error: bool,
    pub content_preview: String,
    pub original_chars: usize,
}

pub(super) struct ScanResult {
    pub calls: Vec<CallMeta>,
    pub outputs: Vec<OutputMeta>,
    /// path -> Vec<(record_idx, source_line)>
    pub file_writes: BTreeMap<String, Vec<(usize, usize)>>,
}

pub(super) fn scan_records(records: &[super::super::Record]) -> ScanResult {
    let mut calls: Vec<CallMeta> = Vec::new();
    let mut outputs: Vec<OutputMeta> = Vec::new();
    let mut file_writes: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();

    for (idx, record) in records.iter().enumerate() {
        let ev = &record.event;

        // Context / Index view
        let kind = ev.get("kind").and_then(Value::as_str);
        if matches!(kind, Some("function_call" | "custom_tool_call")) {
            let name = ev.get("name").and_then(Value::as_str).unwrap_or("");
            let args = ev
                .get("arguments")
                .or_else(|| ev.get("input"))
                .unwrap_or(&Value::Null);
            if let Some(action) = extract_tool_action(name, args) {
                if let ToolAction::WriteFile { ref path } = action {
                    file_writes
                        .entry(path.clone())
                        .or_default()
                        .push((idx, record.line));
                }
                calls.push(CallMeta {
                    record_idx: idx,
                    source_line: record.line,
                    call_id: ev.get("call_id").and_then(Value::as_str).map(str::to_owned),
                    action,
                });
            }
            continue;
        }

        if matches!(
            kind,
            Some("function_call_output" | "custom_tool_call_output")
        ) {
            let exit_code = ev.get("reported_exit_code").and_then(Value::as_i64);
            let is_error = exit_code.is_some_and(|c| c != 0);
            let preview = ev.get("preview").and_then(Value::as_str).unwrap_or("");
            let chars = ev
                .get("chars")
                .and_then(Value::as_u64)
                .unwrap_or(preview.len() as u64) as usize;
            outputs.push(OutputMeta {
                record_idx: idx,
                source_line: record.line,
                call_id: ev.get("call_id").and_then(Value::as_str).map(str::to_owned),
                call_line: ev
                    .get("call_line")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize),
                is_error,
                content_preview: preview.to_string(),
                original_chars: chars,
            });
            continue;
        }

        // Dialogue view
        if ev.get("role").and_then(Value::as_str) == Some("tool") {
            let content = ev.get("content").and_then(Value::as_str).unwrap_or("");
            let chars = content.chars().count();
            outputs.push(OutputMeta {
                record_idx: idx,
                source_line: record.line,
                call_id: None,
                call_line: None,
                is_error: false,
                content_preview: content.chars().take(120).collect(),
                original_chars: chars,
            });
            continue;
        }

        // Tool calls array (Antigravity / Gemini / OpenAI)
        if let Some(tool_calls) = ev.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let cid = tc.get("id").and_then(Value::as_str).map(str::to_owned);
                let name = tc
                    .pointer("/function/name")
                    .or_else(|| tc.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = tc
                    .pointer("/function/arguments")
                    .or_else(|| tc.get("arguments"))
                    .or_else(|| tc.get("args"))
                    .unwrap_or(&Value::Null);
                if let Some(action) = extract_tool_action(name, args) {
                    if let ToolAction::WriteFile { ref path } = action {
                        file_writes
                            .entry(path.clone())
                            .or_default()
                            .push((idx, record.line));
                    }
                    calls.push(CallMeta {
                        record_idx: idx,
                        source_line: record.line,
                        call_id: cid,
                        action,
                    });
                }
            }
            continue;
        }

        // Claude format: message.content array with tool_use or tool_result
        if let Some(parts) = ev
            .pointer("/message/content")
            .or_else(|| ev.get("content"))
            .and_then(Value::as_array)
        {
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let cid = part.get("id").and_then(Value::as_str).map(str::to_owned);
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = part.get("input").unwrap_or(&Value::Null);
                    if let Some(action) = extract_tool_action(name, input) {
                        if let ToolAction::WriteFile { ref path } = action {
                            file_writes
                                .entry(path.clone())
                                .or_default()
                                .push((idx, record.line));
                        }
                        calls.push(CallMeta {
                            record_idx: idx,
                            source_line: record.line,
                            call_id: cid,
                            action,
                        });
                    }
                } else if part.get("type").and_then(Value::as_str) == Some("tool_result") {
                    let cid = part
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let is_error = part
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let text = part.get("content").and_then(Value::as_str).unwrap_or("");
                    outputs.push(OutputMeta {
                        record_idx: idx,
                        source_line: record.line,
                        call_id: cid,
                        call_line: None,
                        is_error,
                        content_preview: text.chars().take(120).collect(),
                        original_chars: text.chars().count(),
                    });
                }
            }
        }

        // Raw events view (Codex payload)
        let p_type = ev.pointer("/payload/type").and_then(Value::as_str);
        if p_type == Some("function_call") {
            let name = ev
                .pointer("/payload/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = ev.pointer("/payload/arguments").unwrap_or(&Value::Null);
            if let Some(action) = extract_tool_action(name, args) {
                if let ToolAction::WriteFile { ref path } = action {
                    file_writes
                        .entry(path.clone())
                        .or_default()
                        .push((idx, record.line));
                }
                calls.push(CallMeta {
                    record_idx: idx,
                    source_line: record.line,
                    call_id: ev
                        .pointer("/payload/call_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    action,
                });
            }
        } else if p_type == Some("function_call_output") {
            let out_str = ev
                .pointer("/payload/output")
                .and_then(Value::as_str)
                .unwrap_or("");
            let chars = out_str.chars().count();
            outputs.push(OutputMeta {
                record_idx: idx,
                source_line: record.line,
                call_id: ev
                    .pointer("/payload/call_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                call_line: None,
                is_error: false,
                content_preview: out_str.chars().take(120).collect(),
                original_chars: chars,
            });
        } else if ev.get("type").and_then(Value::as_str) == Some("USER_INPUT") && !calls.is_empty()
        {
            // In Antigravity / Gemini transcripts, tool output appears in USER_INPUT
            let content_str = ev.get("content").and_then(Value::as_str).unwrap_or("");
            if !content_str.is_empty() {
                let chars = content_str.chars().count();
                outputs.push(OutputMeta {
                    record_idx: idx,
                    source_line: record.line,
                    call_id: None,
                    call_line: None,
                    is_error: false,
                    content_preview: content_str.chars().take(120).collect(),
                    original_chars: chars,
                });
            }
        }
    }

    ScanResult {
        calls,
        outputs,
        file_writes,
    }
}

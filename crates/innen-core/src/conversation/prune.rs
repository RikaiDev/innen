//! Deterministic rule-based tool pruning.
//!
//! Drops or truncates provably stale tool outputs (e.g. file reads superseded by
//! subsequent writes/edits, duplicate identical reads, empty search results)
//! without invoking an external LLM.
//!
//! Preserves user instructions, assistant responses, failing tool outputs,
//! recent turns, and exact line/SHA-256 provenance.

use super::Page;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct PruneOptions {
    /// Number of newest records to unconditionally keep untouched. Default: 4.
    pub preserve_recent: usize,
    /// Truncate preview head characters for retained notes. Default: 120.
    pub truncate_head_chars: usize,
}

impl Default for PruneOptions {
    fn default() -> Self {
        Self {
            preserve_recent: 4,
            truncate_head_chars: 120,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PruneReceipt {
    pub total_records: usize,
    pub pruned_count: usize,
    pub superseded_reads: usize,
    pub duplicate_reads: usize,
    pub empty_searches: usize,
    pub saved_chars_estimate: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolAction {
    ReadFile { path: String },
    WriteFile { path: String },
    Search { query: String },
    StatusCheck { cmd: String },
}

fn normalize_path(raw: &str) -> String {
    let s = raw.trim().trim_matches('\'').trim_matches('"');
    let s = s.strip_prefix("./").unwrap_or(s);
    s.to_string()
}

fn extract_tool_action(name: &str, args_val: &Value) -> Option<ToolAction> {
    let lower_name = name.to_ascii_lowercase();

    let parsed_obj: Option<Value> = if args_val.is_string() {
        serde_json::from_str(args_val.as_str().unwrap()).ok()
    } else if args_val.is_object() {
        Some(args_val.clone())
    } else {
        None
    };

    let get_str = |keys: &[&str]| -> Option<String> {
        let obj = parsed_obj.as_ref()?.as_object()?;
        for k in keys {
            if let Some(s) = obj.get(*k).and_then(Value::as_str) {
                if !s.trim().is_empty() {
                    return Some(s.to_string());
                }
            }
        }
        None
    };

    if ["read_file", "view_file", "view", "read", "cat"].contains(&lower_name.as_str()) {
        if let Some(p) = get_str(&["file_path", "path", "file", "AbsolutePath", "target"]) {
            return Some(ToolAction::ReadFile {
                path: normalize_path(&p),
            });
        }
    }

    if [
        "write_to_file",
        "replace_file_content",
        "edit",
        "write",
        "write_file",
        "apply_patch",
        "str_replace_editor",
    ]
    .contains(&lower_name.as_str())
    {
        if let Some(p) = get_str(&[
            "file_path",
            "path",
            "file",
            "AbsolutePath",
            "TargetFile",
            "target",
        ]) {
            return Some(ToolAction::WriteFile {
                path: normalize_path(&p),
            });
        }
    }

    if [
        "grep_search",
        "find_by_name",
        "file_search",
        "grep",
        "glob",
        "find",
    ]
    .contains(&lower_name.as_str())
    {
        let q = get_str(&["query", "pattern", "Query", "Pattern", "path"]).unwrap_or_default();
        return Some(ToolAction::Search { query: q });
    }

    if ["exec_command", "bash", "sh"].contains(&lower_name.as_str()) {
        if let Some(cmd) = get_str(&["command", "cmd", "CommandLine"]) {
            let trimmed = cmd.trim();
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            if tokens.len() >= 2
                && ["cat", "head", "tail", "bat"].contains(&tokens[0])
                && !tokens[1].starts_with('-')
            {
                return Some(ToolAction::ReadFile {
                    path: normalize_path(tokens[1]),
                });
            }
            if tokens.len() >= 2 && ["rg", "grep"].contains(&tokens[0]) {
                return Some(ToolAction::Search {
                    query: tokens[1].to_string(),
                });
            }
            if trimmed.starts_with("git status") || trimmed.starts_with("git diff") {
                return Some(ToolAction::StatusCheck {
                    cmd: trimmed.to_string(),
                });
            }
        }
    }

    None
}

fn is_empty_search_output(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "[]"
        || lower == "no results found"
        || lower.starts_with("found 0 results")
        || lower.contains("0 matches")
        || lower.contains("no matches found")
}

/// Prune stale tool results in a `Page` deterministically according to rules.
pub fn prune_page(page: &mut Page, options: &PruneOptions) -> PruneReceipt {
    let mut receipt = PruneReceipt {
        total_records: page.records.len(),
        ..Default::default()
    };

    if page.records.is_empty() {
        return receipt;
    }

    let keep_threshold = page.records.len().saturating_sub(options.preserve_recent);

    // Phase 1: Scan and map calls and outputs
    struct CallMeta {
        record_idx: usize,
        source_line: usize,
        call_id: Option<String>,
        action: ToolAction,
    }

    struct OutputMeta {
        record_idx: usize,
        source_line: usize,
        call_id: Option<String>,
        call_line: Option<usize>,
        is_error: bool,
        content_preview: String,
        original_chars: usize,
    }

    let mut calls: Vec<CallMeta> = Vec::new();
    let mut outputs: Vec<OutputMeta> = Vec::new();
    let mut file_writes: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new(); // path -> Vec<(record_idx, source_line)>

    for (idx, record) in page.records.iter().enumerate() {
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

    // Phase 2: Link calls to outputs
    struct PairedTool {
        call_idx: usize,
        output_idx: usize,
        output_line: usize,
        action: ToolAction,
        is_error: bool,
        content_preview: String,
        original_chars: usize,
    }

    let mut paired: Vec<PairedTool> = Vec::new();
    let mut matched_outputs = BTreeSet::new();

    for call in &calls {
        let matching_out = outputs.iter().find(|o| {
            if matched_outputs.contains(&o.record_idx) {
                return false;
            }
            if let (Some(cid1), Some(cid2)) = (&call.call_id, &o.call_id) {
                cid1 == cid2
            } else if let Some(cline) = o.call_line {
                cline == call.source_line
            } else {
                // Adjacent fallback
                o.record_idx > call.record_idx && o.record_idx <= call.record_idx + 2
            }
        });

        if let Some(out) = matching_out {
            matched_outputs.insert(out.record_idx);
            paired.push(PairedTool {
                call_idx: call.record_idx,
                output_idx: out.record_idx,
                output_line: out.source_line,
                action: call.action.clone(),
                is_error: out.is_error,
                content_preview: out.content_preview.clone(),
                original_chars: out.original_chars,
            });
        }
    }

    // Phase 3: Evaluate pruning rules
    // Prune decision: (record_idx, reason_string, chars_saved)
    let mut to_prune: BTreeMap<usize, (String, usize)> = BTreeMap::new();

    // Group file reads by path to detect supersession and redundancy
    let mut path_reads: BTreeMap<String, Vec<&PairedTool>> = BTreeMap::new();
    for tool in &paired {
        if let ToolAction::ReadFile { ref path } = tool.action {
            path_reads.entry(path.clone()).or_default().push(tool);
        }
    }

    // Rule 1 & Rule 2: File Reads
    for (path, reads) in path_reads {
        let writes = file_writes.get(&path);

        for (i, read_tool) in reads.iter().enumerate() {
            // Never prune errors or recent records
            if read_tool.is_error || read_tool.output_idx >= keep_threshold {
                continue;
            }

            // Check if superseded by later write
            if let Some(w_list) = writes {
                if let Some((_, write_line)) = w_list
                    .iter()
                    .find(|(w_idx, _)| *w_idx > read_tool.output_idx)
                {
                    let reason =
                        format!("read of '{path}' superseded by write at line {write_line}");
                    to_prune.insert(read_tool.output_idx, (reason, read_tool.original_chars));
                    receipt.superseded_reads += 1;
                    continue;
                }
            }

            // Check if redundant with a later read of same file (with no writes in between)
            if i + 1 < reads.len() {
                let next_read = reads[i + 1];
                let no_intervening_writes = writes
                    .map(|wl| {
                        !wl.iter().any(|(w_idx, _)| {
                            *w_idx > read_tool.output_idx && *w_idx < next_read.call_idx
                        })
                    })
                    .unwrap_or(true);
                if no_intervening_writes {
                    let reason = format!(
                        "read of '{path}' redundant; re-read at line {}",
                        next_read.output_line
                    );
                    to_prune.insert(read_tool.output_idx, (reason, read_tool.original_chars));
                    receipt.duplicate_reads += 1;
                    continue;
                }
            }
        }
    }

    // Rule 3: Empty Searches
    for tool in &paired {
        if tool.is_error || tool.output_idx >= keep_threshold {
            continue;
        }
        if let ToolAction::Search { ref query } = tool.action {
            if is_empty_search_output(&tool.content_preview) {
                let reason = format!("search for '{query}' returned 0 matches");
                to_prune.insert(tool.output_idx, (reason, tool.original_chars));
                receipt.empty_searches += 1;
            }
        }
    }

    // Phase 4: Apply pruning in-place to page.records
    for (idx, (reason, orig_chars)) in &to_prune {
        let record = &mut page.records[*idx];
        let ev = &mut record.event;

        if let Some(obj) = ev.as_object_mut() {
            obj.insert("pruned".to_string(), json!(true));
            obj.insert("prune_reason".to_string(), json!(reason));

            if obj.contains_key("preview") {
                let note = format!("[omitted: {reason}]");
                let note_len = note.chars().count();
                obj.insert("preview".to_string(), json!(note));
                obj.insert("preview_tail".to_string(), json!(""));
                obj.insert("preview_truncated".to_string(), json!(false));
                obj.insert("chars".to_string(), json!(note_len));
                obj.insert("omitted_chars".to_string(), json!(orig_chars));
            }
            if obj.contains_key("output") {
                let note = format!("[tool output omitted: {reason}]");
                obj.insert("output".to_string(), json!(note));
            }
            if obj.contains_key("content") {
                let note = format!("[tool output omitted: {reason}]");
                obj.insert("content".to_string(), json!(note));
            }
            if let Some(payload) = obj.get_mut("payload").and_then(Value::as_object_mut) {
                if payload.contains_key("output") {
                    payload.insert(
                        "output".to_string(),
                        json!(format!("[tool output omitted: {reason}]")),
                    );
                }
            }
        }

        receipt.pruned_count += 1;
        receipt.saved_chars_estimate += orig_chars;
    }

    if receipt.pruned_count > 0 {
        page.warnings.push(format!(
            "pruned {} stale tool output(s) using deterministic rules (saved ~{} chars)",
            receipt.pruned_count, receipt.saved_chars_estimate
        ));
    }

    receipt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{Page, Record, Source};
    use std::path::PathBuf;

    fn make_page(records: Vec<Record>) -> Page {
        Page {
            session_id: "test-session".into(),
            source: Source::Codex,
            source_path: PathBuf::from("/tmp/test.jsonl"),
            view: "context".into(),
            offset: 0,
            next_offset: None,
            records,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn prune_superseded_read_when_file_subsequently_written() {
        let records = vec![
            Record {
                line: 1,
                event: json!({
                    "kind": "function_call",
                    "name": "read_file",
                    "arguments": "{\"path\": \"src/main.rs\"}",
                }),
            },
            Record {
                line: 2,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 1,
                    "reported_exit_code": 0,
                    "chars": 5000,
                    "preview": "fn main() { println!(\"old\"); }",
                    "preview_tail": "}",
                }),
            },
            Record {
                line: 3,
                event: json!({
                    "kind": "function_call",
                    "name": "write_to_file",
                    "arguments": "{\"path\": \"src/main.rs\"}",
                }),
            },
            Record {
                line: 4,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 3,
                    "reported_exit_code": 0,
                    "preview": "ok wrote 50 bytes",
                }),
            },
            // 4 recent turns kept untouched
            Record {
                line: 5,
                event: json!({"role": "user", "content": "next task"}),
            },
            Record {
                line: 6,
                event: json!({"role": "assistant", "content": "working"}),
            },
            Record {
                line: 7,
                event: json!({"role": "user", "content": "status?"}),
            },
            Record {
                line: 8,
                event: json!({"role": "assistant", "content": "done"}),
            },
        ];

        let mut page = make_page(records);
        let receipt = prune_page(
            &mut page,
            &PruneOptions {
                preserve_recent: 4,
                truncate_head_chars: 120,
            },
        );

        assert_eq!(receipt.pruned_count, 1);
        assert_eq!(receipt.superseded_reads, 1);
        assert_eq!(page.records[1].event["pruned"], true);
        assert!(page.records[1].event["preview"]
            .as_str()
            .unwrap()
            .contains("superseded by write at line 3"));
    }

    #[test]
    fn do_not_prune_failing_tool_reads() {
        let records = vec![
            Record {
                line: 1,
                event: json!({
                    "kind": "function_call",
                    "name": "read_file",
                    "arguments": "{\"path\": \"src/main.rs\"}",
                }),
            },
            Record {
                line: 2,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 1,
                    "reported_exit_code": 1,
                    "preview": "Error: Permission denied",
                }),
            },
            Record {
                line: 3,
                event: json!({
                    "kind": "function_call",
                    "name": "write_to_file",
                    "arguments": "{\"path\": \"src/main.rs\"}",
                }),
            },
            Record {
                line: 4,
                event: json!({"role": "user", "content": "recent"}),
            },
        ];

        let mut page = make_page(records);
        let receipt = prune_page(
            &mut page,
            &PruneOptions {
                preserve_recent: 1,
                truncate_head_chars: 120,
            },
        );

        assert_eq!(receipt.pruned_count, 0);
        assert_eq!(page.records[1].event.get("pruned"), None);
    }

    #[test]
    fn prune_empty_search_output() {
        let records = vec![
            Record {
                line: 1,
                event: json!({
                    "kind": "function_call",
                    "name": "grep_search",
                    "arguments": "{\"query\": \"nonexistent_symbol\"}",
                }),
            },
            Record {
                line: 2,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 1,
                    "reported_exit_code": 0,
                    "chars": 120,
                    "preview": "Found 0 results",
                }),
            },
            Record {
                line: 3,
                event: json!({"role": "user", "content": "1"}),
            },
            Record {
                line: 4,
                event: json!({"role": "assistant", "content": "2"}),
            },
        ];

        let mut page = make_page(records);
        let receipt = prune_page(
            &mut page,
            &PruneOptions {
                preserve_recent: 2,
                truncate_head_chars: 120,
            },
        );

        assert_eq!(receipt.pruned_count, 1);
        assert_eq!(receipt.empty_searches, 1);
        assert_eq!(page.records[1].event["pruned"], true);
    }

    #[test]
    fn prune_measures_token_reduction_on_typical_workload() {
        let file_a_body =
            "pub fn calculate_metric(x: f64) -> f64 {\n    x * 2.0 + 1.0\n}\n".repeat(100);
        let records = vec![
            Record {
                line: 1,
                event: json!({
                    "kind": "function_call",
                    "name": "read_file",
                    "arguments": "{\"path\": \"src/metric.rs\"}",
                }),
            },
            Record {
                line: 2,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 1,
                    "reported_exit_code": 0,
                    "chars": file_a_body.len(),
                    "output": file_a_body,
                }),
            },
            Record {
                line: 3,
                event: json!({
                    "kind": "function_call",
                    "name": "write_to_file",
                    "arguments": "{\"path\": \"src/metric.rs\"}",
                }),
            },
            Record {
                line: 4,
                event: json!({
                    "kind": "function_call_output",
                    "call_line": 3,
                    "reported_exit_code": 0,
                    "output": "ok wrote 120 bytes",
                }),
            },
            // Recent turns
            Record {
                line: 5,
                event: json!({"role": "user", "content": "run tests"}),
            },
            Record {
                line: 6,
                event: json!({"role": "assistant", "content": "all tests passed"}),
            },
            Record {
                line: 7,
                event: json!({"role": "user", "content": "next task"}),
            },
            Record {
                line: 8,
                event: json!({"role": "assistant", "content": "ready"}),
            },
        ];

        let mut page = make_page(records);
        let before_json = serde_json::to_string(&page).unwrap();
        let before_tokens = crate::conversation::grammar::tokens_text(&before_json).unwrap();

        let receipt = prune_page(
            &mut page,
            &PruneOptions {
                preserve_recent: 4,
                truncate_head_chars: 120,
            },
        );
        let after_json = serde_json::to_string(&page).unwrap();
        let after_tokens = crate::conversation::grammar::tokens_text(&after_json).unwrap();

        assert_eq!(receipt.pruned_count, 1);
        assert_eq!(receipt.superseded_reads, 1);
        assert!(after_tokens < before_tokens);
        println!(
            "Token comparison: before={} tokens, after={} tokens, saved={} tokens ({:.2}%)",
            before_tokens,
            after_tokens,
            before_tokens - after_tokens,
            (1.0 - (after_tokens as f64 / before_tokens as f64)) * 100.0
        );
    }
}

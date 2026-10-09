//! Tool-action classification: what a raw tool call was asking for.
//!
//! Pruning rules are only as trustworthy as this layer. It reads a call's name
//! and arguments across the transcript dialects and reduces them to one
//! `ToolAction`, so the rules never re-parse vendor payload shapes.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ToolAction {
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

pub(super) fn extract_tool_action(name: &str, args_val: &Value) -> Option<ToolAction> {
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

pub(super) fn is_empty_search_output(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "[]"
        || lower == "no results found"
        || lower.starts_with("found 0 results")
        || lower.contains("0 matches")
        || lower.contains("no matches found")
}

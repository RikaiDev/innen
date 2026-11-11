//! Code change extraction from tool calls.

use super::discourse::DiscourseUnit;
use std::collections::BTreeMap;

struct CodeChangeBuilder {
    files: Vec<String>,
    change_type: super::CodeChangeType,
    summary: String,
    source_lines: Vec<usize>,
    completed: bool,
    related_decision: Option<String>,
    diff_summary: Option<String>,
    tool_calls: Vec<String>,
}

/// Extract code changes from tool calls (highest fidelity signal).
pub fn extract_code_changes_from_tool_calls(units: &[DiscourseUnit]) -> Vec<super::CodeChange> {
    let mut file_changes: BTreeMap<String, CodeChangeBuilder> = BTreeMap::new();

    for unit in units {
        for tool_call in &unit.tool_calls {
            if let Some(file_path) =
                extract_file_path_from_tool_call(&tool_call.name, &tool_call.args)
            {
                let change_type = classify_change_type_from_tool(&tool_call.name, &tool_call.args);
                let builder = file_changes
                    .entry(file_path.clone())
                    .or_insert(CodeChangeBuilder {
                        files: vec![file_path.clone()],
                        change_type,
                        summary: String::new(),
                        source_lines: Vec::new(),
                        completed: false,
                        related_decision: None,
                        diff_summary: None,
                        tool_calls: Vec::new(),
                    });
                builder.source_lines.push(unit.line);
                builder.tool_calls.push(tool_call.name.clone());
                if builder.summary.is_empty() {
                    builder.summary = format!("{} {}", change_type_str(change_type), file_path);
                }
            }
        }

        for tool_result in &unit.tool_results {
            if let Some(file_path) =
                extract_file_path_from_tool_result(&tool_result.name, &tool_result.output)
            {
                if let Some(builder) = file_changes.get_mut(&file_path) {
                    builder.completed = builder.completed || tool_result.success;
                    if tool_result.success && tool_result.output.contains("diff") {
                        builder.diff_summary = Some(extract_diff_summary(&tool_result.output));
                    }
                }
            }
        }
    }

    file_changes
        .into_values()
        .filter(|b| !b.files.is_empty())
        .map(|b| super::CodeChange {
            files: b.files,
            change_type: b.change_type,
            summary: truncate(&b.summary, 300),
            source_lines: b.source_lines,
            completed: b.completed,
            related_decision: b.related_decision,
            diff_summary: b.diff_summary,
            tool_calls: b.tool_calls,
        })
        .collect()
}

fn extract_file_path_from_tool_call(name: &str, args: &serde_json::Value) -> Option<String> {
    match name {
        "read" | "write" | "edit" | "bash" | "glob" | "grep" | "task" | "ls" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn extract_file_path_from_tool_result(name: &str, output: &str) -> Option<String> {
    if name == "write" || name == "edit" || name == "bash" {
        let regex = regex::Regex::new(
            r"([\w\-./]+\.(?:rs|ts|tsx|js|py|json|toml|yaml|yml|md|sql|sh|css|html))",
        )
        .ok();
        if let Some(re) = regex {
            if let Some(mat) = re.find(output) {
                return Some(mat.as_str().to_string());
            }
        }
    }
    None
}

fn classify_change_type_from_tool(name: &str, args: &serde_json::Value) -> super::CodeChangeType {
    match name {
        "write" => super::CodeChangeType::Create,
        "edit" => super::CodeChangeType::Modify,
        "bash" => {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let cmd_lower = cmd.to_lowercase();
                if cmd_lower.contains("rm ") || cmd_lower.contains("delete") {
                    super::CodeChangeType::Delete
                } else if cmd_lower.contains("test") {
                    super::CodeChangeType::Test
                } else if cmd_lower.contains("cargo") || cmd_lower.contains("build") {
                    super::CodeChangeType::Config
                } else {
                    super::CodeChangeType::Other
                }
            } else {
                super::CodeChangeType::Other
            }
        }
        _ => super::CodeChangeType::Other,
    }
}

fn change_type_str(ct: super::CodeChangeType) -> &'static str {
    match ct {
        super::CodeChangeType::Create => "Create",
        super::CodeChangeType::Modify => "Modify",
        super::CodeChangeType::Delete => "Delete",
        super::CodeChangeType::Refactor => "Refactor",
        super::CodeChangeType::Fix => "Fix",
        super::CodeChangeType::Test => "Test",
        super::CodeChangeType::Config => "Config",
        super::CodeChangeType::Format => "Format",
        super::CodeChangeType::Security => "Security",
        super::CodeChangeType::Compliance => "Compliance",
        super::CodeChangeType::Other => "Change",
    }
}

fn extract_diff_summary(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let mut diff_lines = Vec::new();
    let mut in_diff = false;
    for line in &lines {
        if line.starts_with("```diff")
            || line.starts_with("diff ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            in_diff = true;
        }
        if in_diff {
            diff_lines.push(*line);
            if diff_lines.len() > 20 {
                break;
            }
        }
    }
    if diff_lines.is_empty() {
        for line in &lines {
            if line.starts_with('+') || line.starts_with('-') {
                diff_lines.push(*line);
                if diff_lines.len() > 20 {
                    break;
                }
            }
        }
    }
    truncate(&diff_lines.join("\n"), 500)
}

/// Extract code state diff from tool calls.
pub fn extract_code_state_diff_from_tool_calls(units: &[DiscourseUnit]) -> Option<String> {
    let mut diffs = Vec::new();
    for unit in units.iter().rev().take(50) {
        for tool_result in &unit.tool_results {
            if tool_result.success
                && (tool_result.output.contains("diff") || tool_result.output.contains("patch"))
            {
                diffs.push(truncate(&tool_result.output, 300));
            }
        }
    }
    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join(" | "))
    }
}

fn truncate(s: &str, max: usize) -> String {
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

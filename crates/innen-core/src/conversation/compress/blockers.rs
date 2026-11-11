//! Blocker extraction from conversation structure.

use super::discourse::DiscourseUnit;

/// Extract blockers from structure: failed tool calls + explicit statements.
pub fn extract_blockers_from_structure(units: &[DiscourseUnit]) -> Vec<super::Blocker> {
    let mut blockers = Vec::new();

    // 1. Failed tool calls are primary blocker signals
    for unit in units {
        for tool_result in &unit.tool_results {
            if !tool_result.success {
                let category = categorize_blocker_from_tool(&tool_result.output);
                let severity = estimate_blocker_severity(&tool_result.output, category);
                blockers.push(super::Blocker {
                    description: format!(
                        "Tool '{}' failed: {}",
                        tool_result.name,
                        truncate(&tool_result.output, 200)
                    ),
                    category,
                    resolved: false,
                    resolution: None,
                    source_lines: vec![unit.line],
                    severity,
                    blocking: extract_blocking_target(&tool_result.output),
                    failed_tool_calls: vec![tool_result.name.clone()],
                });
            }
        }
    }

    // 2. Explicit blocker statements from any role
    let blocker_markers = [
        ("blocked", super::BlockerCategory::Technical),
        ("stuck", super::BlockerCategory::Technical),
        ("cannot", super::BlockerCategory::Technical),
        ("unable", super::BlockerCategory::Technical),
        ("waiting for", super::BlockerCategory::Dependency),
        ("depends on", super::BlockerCategory::Dependency),
        ("permission", super::BlockerCategory::Permission),
        ("access denied", super::BlockerCategory::Permission),
        ("missing", super::BlockerCategory::Information),
        ("unknown", super::BlockerCategory::Information),
        ("timeout", super::BlockerCategory::Resource),
        ("limit", super::BlockerCategory::Resource),
        ("compliance", super::BlockerCategory::Compliance),
        ("legal", super::BlockerCategory::Compliance),
        ("privacy", super::BlockerCategory::Compliance),
        ("個資", super::BlockerCategory::Compliance),
        ("合法", super::BlockerCategory::Compliance),
        ("合規", super::BlockerCategory::Compliance),
        ("資安", super::BlockerCategory::Compliance),
    ];

    for unit in units {
        let content_lower = unit.content.to_lowercase();
        for (marker, category) in &blocker_markers {
            if content_lower.contains(marker) && is_blocker_statement(&content_lower) {
                let resolved = check_blocker_resolution(units, unit.line);
                let resolution = if resolved {
                    extract_resolution_from_context(units, unit.line)
                } else {
                    None
                };
                blockers.push(super::Blocker {
                    description: truncate(&unit.content, 300),
                    category: *category,
                    resolved,
                    resolution,
                    source_lines: vec![unit.line],
                    severity: estimate_blocker_severity(&content_lower, *category),
                    blocking: extract_blocking_target(&content_lower),
                    failed_tool_calls: vec![],
                });
                break;
            }
        }
    }

    blockers.sort_by(|a, b| b.severity.partial_cmp(&a.severity).unwrap());
    blockers.dedup_by(|a, b| a.description == b.description);
    blockers
}

fn categorize_blocker_from_tool(output: &str) -> super::BlockerCategory {
    let output_lower = output.to_lowercase();
    if output_lower.contains("permission") || output_lower.contains("access denied") {
        super::BlockerCategory::Permission
    } else if output_lower.contains("not found") || output_lower.contains("missing") {
        super::BlockerCategory::Dependency
    } else if output_lower.contains("timeout") {
        super::BlockerCategory::Resource
    } else if output_lower.contains("compliance")
        || output_lower.contains("privacy")
        || output_lower.contains("個資")
    {
        super::BlockerCategory::Compliance
    } else {
        super::BlockerCategory::Technical
    }
}

fn is_blocker_statement(text: &str) -> bool {
    let patterns = [
        "blocked by",
        "blocked on",
        "stuck on",
        "cannot proceed",
        "unable to",
        "waiting for",
        "depends on",
        "阻擋",
        "卡住",
        "無法",
        "等待",
        "依賴",
    ];
    patterns.iter().any(|p| text.contains(p))
}

fn check_blocker_resolution(units: &[DiscourseUnit], blocker_line: usize) -> bool {
    let resolve_markers = [
        "resolved",
        "fixed",
        "solved",
        "worked around",
        "bypassed",
        "unblocked",
        "completed",
        "done",
        "merged",
        "解決",
        "修復",
        "繞過",
        "完成",
    ];
    let blocker_idx = units
        .iter()
        .position(|u| u.line == blocker_line)
        .unwrap_or(0);
    let end = (blocker_idx + 15).min(units.len());
    for unit in &units[blocker_idx..end] {
        let content_lower = unit.content.to_lowercase();
        if resolve_markers.iter().any(|m| content_lower.contains(m)) {
            return true;
        }
    }
    false
}

fn extract_resolution_from_context(units: &[DiscourseUnit], blocker_line: usize) -> Option<String> {
    let resolve_markers = [
        "resolved",
        "fixed",
        "solved",
        "worked around",
        "bypassed",
        "unblocked",
        "completed",
        "done",
        "merged",
        "解決",
        "修復",
        "繞過",
        "完成",
    ];
    let blocker_idx = units
        .iter()
        .position(|u| u.line == blocker_line)
        .unwrap_or(0);
    let end = (blocker_idx + 15).min(units.len());
    for unit in &units[blocker_idx..end] {
        let content_lower = unit.content.to_lowercase();
        if resolve_markers.iter().any(|m| content_lower.contains(m)) {
            return Some(truncate(&unit.content, 300));
        }
    }
    None
}

fn extract_blocking_target(text: &str) -> Option<String> {
    let patterns = [
        "blocked on ",
        "blocked by ",
        "stuck on ",
        "waiting for ",
        "depends on ",
    ];
    for pattern in &patterns {
        if let Some(pos) = text.find(pattern) {
            let after = &text[pos + pattern.len()..];
            let end = after
                .find(['.', '。', ',', '，', '\n'])
                .unwrap_or(after.len());
            return Some(truncate(&after[..end], 100));
        }
    }
    None
}

fn estimate_blocker_severity(text: &str, category: super::BlockerCategory) -> f32 {
    let base = match category {
        super::BlockerCategory::Technical => 0.6,
        super::BlockerCategory::Dependency => 0.7,
        super::BlockerCategory::Permission => 0.8,
        super::BlockerCategory::Information => 0.4,
        super::BlockerCategory::Design => 0.5,
        super::BlockerCategory::Resource => 0.5,
        super::BlockerCategory::Compliance => 0.9,
        super::BlockerCategory::Other => 0.3,
    };
    let boost = if text.contains("critical")
        || text.contains("blocking")
        || text.contains("urgent")
        || text.contains("關鍵")
        || text.contains("緊急")
        || text.contains("嚴重")
    {
        0.2
    } else {
        0.0
    };
    let result = base + boost;
    if result > 1.0 {
        1.0
    } else {
        result
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

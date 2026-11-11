//! User requirements and deliverables extraction.

use super::discourse::{DiscourseUnit, Role};

/// Extract user requirements from user messages.
pub fn extract_user_requirements_from_user_messages(
    units: &[DiscourseUnit],
) -> Vec<super::UserRequirement> {
    let mut requirements = Vec::new();

    let requirement_signals = [
        ("must", super::RequirementCategory::Content, true),
        ("have to", super::RequirementCategory::Content, true),
        ("need to", super::RequirementCategory::Content, true),
        ("required", super::RequirementCategory::Content, true),
        ("constraint", super::RequirementCategory::Technical, true),
        ("requirement", super::RequirementCategory::Content, true),
        ("format", super::RequirementCategory::Format, false),
        ("style", super::RequirementCategory::Format, false),
        ("audience", super::RequirementCategory::Audience, false),
        ("timeline", super::RequirementCategory::Timeline, false),
        ("deadline", super::RequirementCategory::Timeline, true),
        ("compliance", super::RequirementCategory::Compliance, true),
        ("legal", super::RequirementCategory::Compliance, true),
        ("privacy", super::RequirementCategory::Compliance, true),
        ("個資", super::RequirementCategory::Compliance, true),
        ("合法", super::RequirementCategory::Compliance, true),
        ("合規", super::RequirementCategory::Compliance, true),
        ("可理解", super::RequirementCategory::Audience, false),
        ("說服", super::RequirementCategory::Audience, false),
        ("無資訊背景", super::RequirementCategory::Audience, false),
        ("看得懂", super::RequirementCategory::Audience, false),
    ];

    for unit in units {
        if unit.role != Role::User {
            continue;
        }
        let content_lower = unit.content.to_lowercase();

        for (signal, category, hard) in &requirement_signals {
            if content_lower.contains(signal) {
                let related_deliverable = find_related_deliverable(&unit.content);
                requirements.push(super::UserRequirement {
                    requirement: truncate(&unit.content, 500),
                    source_line: unit.line,
                    hard_constraint: *hard,
                    category: *category,
                    related_deliverable,
                });
            }
        }
    }
    requirements
}

fn find_related_deliverable(text: &str) -> Option<String> {
    let deliverable_markers = [
        "deliver",
        "output",
        "produce",
        "create",
        "make",
        "generate",
        "build",
        "簡報",
        "投影片",
        "報告",
        "文件",
        "script",
        "腳本",
    ];
    let sentences: Vec<&str> = text.split(['.', '。', '!', '！', '?', '？']).collect();
    for sentence in sentences {
        let lower = sentence.to_lowercase();
        if deliverable_markers.iter().any(|m| lower.contains(m)) {
            return Some(truncate(sentence.trim(), 200));
        }
    }
    None
}

/// Extract deliverables from structure: user requests + tool outputs.
pub fn extract_deliverables_from_structure(units: &[DiscourseUnit]) -> Vec<super::Deliverable> {
    let mut deliverables = Vec::new();

    let deliverable_markers = [
        ("deliver", "deliverable"),
        ("output", "output"),
        ("produce", "produce"),
        ("create", "create"),
        ("make", "make"),
        ("generate", "generate"),
        ("build", "build"),
        ("簡報", "presentation"),
        ("投影片", "slides"),
        ("報告", "report"),
        ("文件", "document"),
        ("script", "script"),
        ("腳本", "script"),
    ];

    for unit in units {
        if unit.role != Role::User {
            continue;
        }
        let content_lower = unit.content.to_lowercase();

        for (marker, _dtype) in &deliverable_markers {
            if content_lower.contains(marker) {
                let format = extract_format_from_text(&unit.content);
                let related_requirement = find_related_requirement(&unit.content);
                deliverables.push(super::Deliverable {
                    description: truncate(&unit.content, 500),
                    format,
                    source_line: unit.line,
                    confirmed: content_lower.contains("confirm") || content_lower.contains("確認"),
                    related_requirement,
                });
            }
        }
    }

    for unit in units {
        if unit.role == Role::Assistant {
            for tool_result in &unit.tool_results {
                if tool_result.success
                    && (tool_result.name == "write" || tool_result.name == "bash")
                {
                    if let Some(format) = extract_format_from_text(&tool_result.output) {
                        deliverables.push(super::Deliverable {
                            description: format!(
                                "Generated via {}: {}",
                                tool_result.name,
                                truncate(&tool_result.output, 200)
                            ),
                            format: Some(format),
                            source_line: unit.line,
                            confirmed: true,
                            related_requirement: None,
                        });
                    }
                }
            }
        }
    }
    deliverables
}

fn extract_format_from_text(text: &str) -> Option<String> {
    let formats = [
        "pptx",
        "pdf",
        "google slides",
        "openslide",
        "html",
        "markdown",
        "docx",
        "xlsx",
        "png",
        "svg",
        "json",
        "yaml",
        "yml",
    ];
    let text_lower = text.to_lowercase();
    for fmt in &formats {
        if text_lower.contains(fmt) {
            return Some(fmt.to_string());
        }
    }
    None
}

fn find_related_requirement(text: &str) -> Option<String> {
    let req_markers = [
        "must",
        "need to",
        "required",
        "constraint",
        "requirement",
        "必須",
        "需要",
        "要求",
    ];
    let sentences: Vec<&str> = text.split(['.', '。', '!', '！', '?', '？']).collect();
    for sentence in sentences {
        let lower = sentence.to_lowercase();
        if req_markers.iter().any(|m| lower.contains(m)) {
            return Some(truncate(sentence.trim(), 200));
        }
    }
    None
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

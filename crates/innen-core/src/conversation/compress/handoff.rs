//! Handoff instruction extraction from conversation structure.

use super::discourse::{DiscourseUnit, Role};
use crate::conversation::checkpoint::Checkpoint;

/// Extract handoff instructions from structure: user directives + checkpoint next_action.
pub fn extract_handoff_from_structure(
    units: &[DiscourseUnit],
    latest_checkpoint: Option<&Checkpoint>,
) -> Vec<super::HandoffInstruction> {
    let mut instructions = Vec::new();

    let handoff_markers = [
        ("next step", super::HandoffType::ContinueTask, 0.85, true),
        ("next steps", super::HandoffType::ContinueTask, 0.85, true),
        ("continue", super::HandoffType::ContinueTask, 0.7, true),
        ("then", super::HandoffType::ContinueTask, 0.6, false),
        ("verify", super::HandoffType::VerifyChange, 0.85, true),
        ("test", super::HandoffType::RunTests, 0.8, true),
        ("deploy", super::HandoffType::Deploy, 0.85, true),
        ("review", super::HandoffType::Review, 0.7, true),
        ("document", super::HandoffType::UpdateDocs, 0.6, true),
        ("fix", super::HandoffType::ResolveBlocker, 0.85, true),
        ("resolve", super::HandoffType::ResolveBlocker, 0.85, true),
        ("redesign", super::HandoffType::Redesign, 0.95, true),
        ("重做", super::HandoffType::Redesign, 0.95, true),
        ("重新設計", super::HandoffType::Redesign, 0.95, true),
        ("下一步", super::HandoffType::ContinueTask, 0.85, true),
        ("接著", super::HandoffType::ContinueTask, 0.7, false),
        ("驗證", super::HandoffType::VerifyChange, 0.85, true),
        ("測試", super::HandoffType::RunTests, 0.8, true),
        ("部署", super::HandoffType::Deploy, 0.85, true),
        ("審查", super::HandoffType::Review, 0.7, true),
    ];

    let check_start = if units.len() > 80 {
        units.len() - 80
    } else {
        0
    };

    for unit in &units[check_start..] {
        if unit.role != Role::User {
            continue;
        }
        let content_lower = unit.content.to_lowercase();

        for (marker, inst_type, base_priority, explicit) in &handoff_markers {
            if content_lower.contains(marker) && is_handoff_statement(&content_lower) {
                let resolves_blocker = find_related_blocker(&content_lower);
                instructions.push(super::HandoffInstruction {
                    instruction: extract_handoff_text(&unit.content, marker),
                    priority: *base_priority,
                    context: Some(truncate(&unit.content, 500)),
                    source_lines: vec![unit.line],
                    instruction_type: *inst_type,
                    explicit: *explicit,
                    resolves_blocker,
                });
                break;
            }
        }
    }

    if let Some(cp) = latest_checkpoint {
        if !cp.next_action.is_empty() && cp.next_action != "none" {
            instructions.push(super::HandoffInstruction {
                instruction: cp.next_action.clone(),
                priority: 0.8,
                context: Some(cp.verification.clone()),
                source_lines: vec![],
                instruction_type: super::HandoffType::ContinueTask,
                explicit: true,
                resolves_blocker: None,
            });
        }
    }

    instructions.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap());
    instructions.dedup_by(|a, b| a.instruction == b.instruction);
    instructions
}

fn is_handoff_statement(text: &str) -> bool {
    let patterns = [
        "next step",
        "next steps",
        "continue with",
        "should",
        "need to",
        "must",
        "verify that",
        "test that",
        "deploy to",
        "review the",
        "document the",
        "fix the",
        "resolve the",
        "redesign",
        "重做",
        "重新設計",
        "下一步",
        "接著",
        "驗證",
        "測試",
        "部署",
        "審查",
    ];
    patterns.iter().any(|p| text.contains(p))
}

fn find_related_blocker(text: &str) -> Option<String> {
    let blocker_markers = [
        "blocked", "blocker", "stuck", "failed", "error", "阻擋", "卡住", "失敗", "錯誤",
    ];
    for marker in &blocker_markers {
        if text.contains(marker) {
            return Some(marker.to_string());
        }
    }
    None
}

fn extract_handoff_text(content: &str, marker: &str) -> String {
    let sentences: Vec<&str> = content.split(['.', '。', '!', '！', '?', '？']).collect();
    for sentence in sentences {
        if sentence.to_lowercase().contains(marker) {
            return truncate(sentence.trim(), 300);
        }
    }
    truncate(content, 300)
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

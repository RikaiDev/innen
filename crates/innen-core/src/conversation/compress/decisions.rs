//! Decision extraction from conversation structure.

use super::discourse::{DiscourseUnit, Role};
use super::TaskStateSummary;

/// Extract task state from conversation structure (checkpoint + task_state events).
pub fn extract_task_state_from_structure(
    units: &[DiscourseUnit],
    latest_checkpoint: Option<&crate::conversation::checkpoint::Checkpoint>,
) -> Option<TaskStateSummary> {
    // Priority 1: explicit task_state in tool results or events
    for unit in units.iter().rev() {
        for tool_result in &unit.tool_results {
            if tool_result.name == "task_state" || tool_result.name.contains("task") {
                if let Ok(state) = serde_json::from_str::<crate::conversation::task_state::TaskState>(
                    &tool_result.output,
                ) {
                    return Some(convert_task_state(&state));
                }
            }
        }
        if let Some(task_state) = unit.raw_event.get("task_state") {
            if let Ok(state) = serde_json::from_value::<crate::conversation::task_state::TaskState>(
                task_state.clone(),
            ) {
                return Some(convert_task_state(&state));
            }
        }
    }

    // Priority 2: checkpoint data
    if let Some(cp) = latest_checkpoint {
        return Some(TaskStateSummary {
            objective: Some(cp.objective.clone()),
            completed_work: cp.completed_work.clone(),
            current_evidence: cp.current_evidence.clone(),
            blockers: cp.blockers.clone(),
            next_action: Some(cp.next_action.clone()),
            verification: Some(cp.verification.clone()),
        });
    }

    // Priority 3: infer from conversation flow
    infer_task_state_from_flow(units)
}

fn convert_task_state(state: &crate::conversation::task_state::TaskState) -> TaskStateSummary {
    let completed_work = state
        .items
        .iter()
        .filter(|w| w.status == crate::conversation::task_state::TaskItemStatus::Completed)
        .map(|w| w.content.clone())
        .collect::<Vec<_>>();

    let blockers = state
        .items
        .iter()
        .filter(|b| {
            b.status == crate::conversation::task_state::TaskItemStatus::InProgress
                || b.status == crate::conversation::task_state::TaskItemStatus::Pending
        })
        .map(|b| b.content.clone())
        .collect::<Vec<_>>();

    let next_action = state.next_pending().map(|item| item.content.clone());

    TaskStateSummary {
        objective: None, // TaskState doesn't have explicit objective
        completed_work,
        current_evidence: Vec::new(),
        blockers,
        next_action,
        verification: None,
    }
}

fn infer_task_state_from_flow(units: &[DiscourseUnit]) -> Option<TaskStateSummary> {
    let mut summary = TaskStateSummary {
        objective: None,
        completed_work: Vec::new(),
        current_evidence: Vec::new(),
        blockers: Vec::new(),
        next_action: None,
        verification: None,
    };

    for unit in units.iter().rev().take(50) {
        let content = &unit.content;

        if summary.objective.is_none() && unit.role == Role::User {
            if content.contains("objective")
                || content.contains("goal")
                || content.contains("task")
                || content.contains("need to")
                || content.contains("目標")
                || content.contains("任務")
                || content.contains("需要")
            {
                summary.objective = Some(truncate(content, 300));
            }
        }

        if unit.role == Role::Assistant {
            if content.contains("done")
                || content.contains("completed")
                || content.contains("finished")
                || content.contains("完成")
                || content.contains("做完")
            {
                summary.completed_work.push(truncate(content, 200));
            }
            if content.contains("output")
                || content.contains("result")
                || content.contains("created")
                || content.contains("generated")
                || content.contains("輸出")
                || content.contains("結果")
            {
                summary.current_evidence.push(truncate(content, 200));
            }
            if summary.next_action.is_none()
                && (content.contains("next")
                    || content.contains("then")
                    || content.contains("will")
                    || content.contains("下一步")
                    || content.contains("接著"))
            {
                summary.next_action = Some(truncate(content, 200));
            }
        }

        if content.contains("blocked")
            || content.contains("stuck")
            || content.contains("failed")
            || content.contains("error")
            || content.contains("阻擋")
            || content.contains("卡住")
            || content.contains("失敗")
        {
            summary.blockers.push(truncate(content, 200));
        }

        if summary.verification.is_none()
            && (content.contains("verify")
                || content.contains("test")
                || content.contains("check")
                || content.contains("驗證")
                || content.contains("測試"))
        {
            summary.verification = Some(truncate(content, 200));
        }
    }

    summary.completed_work.dedup();
    summary.current_evidence.dedup();
    summary.blockers.dedup();

    if summary.objective.is_some()
        || summary.next_action.is_some()
        || !summary.completed_work.is_empty()
        || !summary.current_evidence.is_empty()
        || !summary.blockers.is_empty()
        || summary.verification.is_some()
    {
        Some(summary)
    } else {
        None
    }
}

/// Extract decisions from conversation structure.
pub fn extract_decisions_from_structure(
    units: &[DiscourseUnit],
    task_state: &Option<TaskStateSummary>,
) -> Vec<super::Decision> {
    let mut decisions = Vec::new();

    for (idx, unit) in units.iter().enumerate() {
        if unit.role != Role::Assistant {
            continue;
        }

        let content = &unit.content;
        let content_lower = content.to_lowercase();

        let decision_triggers = [
            ("decided", super::DecisionCategory::Implementation),
            ("decision", super::DecisionCategory::Implementation),
            ("choose", super::DecisionCategory::Implementation),
            ("chose", super::DecisionCategory::Implementation),
            ("selected", super::DecisionCategory::Implementation),
            ("will use", super::DecisionCategory::Implementation),
            ("going to use", super::DecisionCategory::Implementation),
            ("approach is", super::DecisionCategory::Implementation),
            ("strategy is", super::DecisionCategory::Implementation),
            ("architecture", super::DecisionCategory::Architecture),
            ("architect", super::DecisionCategory::Architecture),
            ("design", super::DecisionCategory::Design),
            ("設計", super::DecisionCategory::Design),
            ("架構", super::DecisionCategory::Architecture),
            ("tool", super::DecisionCategory::Tooling),
            ("使用", super::DecisionCategory::Tooling),
            ("format", super::DecisionCategory::Format),
            ("pptx", super::DecisionCategory::Format),
            ("pdf", super::DecisionCategory::Format),
            ("security", super::DecisionCategory::Security),
            ("compliance", super::DecisionCategory::Security),
            ("個資", super::DecisionCategory::Security),
            ("合法", super::DecisionCategory::Security),
            ("合規", super::DecisionCategory::Security),
            ("performance", super::DecisionCategory::Performance),
            ("process", super::DecisionCategory::Process),
        ];

        for (trigger, category) in &decision_triggers {
            if content_lower.contains(trigger) && is_decision_statement(&content_lower) {
                let summary = extract_decision_summary(content, trigger);
                let rationale = extract_rationale_from_context(units, idx);
                let alternatives = extract_alternatives_from_context(units, idx);
                let confirmed = check_user_confirmation(units, idx);
                let trigger_context = extract_trigger_context(units, idx);

                decisions.push(super::Decision {
                    summary: truncate(&summary, 300),
                    rationale,
                    source_lines: vec![unit.line],
                    confidence: if confirmed { 0.9 } else { 0.7 },
                    category: *category,
                    confirmed,
                    alternatives,
                    trigger: trigger_context,
                });
                break;
            }
        }
    }

    if let Some(cp) = task_state {
        if let Some(next_action) = &cp.next_action {
            if !next_action.is_empty() {
                decisions.push(super::Decision {
                    summary: format!("Next action: {}", next_action),
                    rationale: cp.verification.clone(),
                    source_lines: vec![],
                    confidence: 0.8,
                    category: super::DecisionCategory::Process,
                    confirmed: true,
                    alternatives: vec![],
                    trigger: Some("checkpoint".to_string()),
                });
            }
        }
    }

    decisions.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    decisions.dedup_by(|a, b| a.summary == b.summary);
    decisions
}

fn is_decision_statement(text: &str) -> bool {
    let decision_markers = [
        "decided",
        "will use",
        "going to",
        "approach is",
        "strategy is",
        "chose",
        "selected",
        "我們決定",
        "我決定",
        "採用",
        "方案是",
    ];
    let question_markers = [
        "?",
        "？",
        "whether",
        "if ",
        "maybe",
        "perhaps",
        "或許",
        "也許",
        "會不會",
    ];
    let has_decision = decision_markers.iter().any(|m| text.contains(m));
    let has_question = question_markers.iter().any(|m| text.contains(m));
    has_decision && !has_question
}

fn extract_decision_summary(content: &str, trigger: &str) -> String {
    let sentences: Vec<&str> = content.split(['.', '。', '!', '！', '?', '？']).collect();
    for sentence in sentences {
        if sentence.to_lowercase().contains(trigger) {
            return sentence.trim().to_string();
        }
    }
    truncate(content, 300)
}

fn extract_rationale_from_context(units: &[DiscourseUnit], decision_idx: usize) -> Option<String> {
    let rationale_markers = [
        "because", "since", "reason", "因為", "所以", "原因", "基於", "由於",
    ];
    for offset in 0..=2 {
        let idx = if decision_idx >= offset {
            decision_idx - offset
        } else {
            continue;
        };
        let unit = &units[idx];
        let content_lower = unit.content.to_lowercase();
        if rationale_markers.iter().any(|m| content_lower.contains(m)) {
            return Some(truncate(&unit.content, 500));
        }
    }
    None
}

fn extract_alternatives_from_context(units: &[DiscourseUnit], decision_idx: usize) -> Vec<String> {
    let mut alternatives = Vec::new();
    let alt_markers = [
        "alternative",
        "option",
        "instead",
        "or ",
        "或者",
        "替代",
        "備選",
        "另一個",
        "另外",
    ];
    let start = decision_idx.saturating_sub(3);
    let end = (decision_idx + 3).min(units.len());
    for unit in &units[start..end] {
        let content_lower = unit.content.to_lowercase();
        if alt_markers.iter().any(|m| content_lower.contains(m)) {
            alternatives.push(truncate(&unit.content, 200));
        }
    }
    alternatives
}

fn check_user_confirmation(units: &[DiscourseUnit], decision_idx: usize) -> bool {
    let confirm_markers = [
        "yes",
        "confirm",
        "confirmed",
        "agreed",
        "ok",
        "good",
        "sounds good",
        "好",
        "確認",
        "同意",
        "沒問題",
        "沒意見",
        "可以",
        "行",
    ];
    let end = (decision_idx + 10).min(units.len());
    for unit in &units[decision_idx..end] {
        if unit.role == Role::User {
            let content_lower = unit.content.to_lowercase();
            if confirm_markers.iter().any(|m| content_lower.contains(m)) {
                return true;
            }
        }
    }
    false
}

fn extract_trigger_context(units: &[DiscourseUnit], decision_idx: usize) -> Option<String> {
    for i in (0..decision_idx).rev() {
        if units[i].role == Role::User {
            return Some(truncate(&units[i].content, 200));
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

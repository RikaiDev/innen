//! Scoring functions for unfinished work and handoff priority.

use super::ExtractionResult;

/// Calculate unfinished score from extraction (0.0-1.0, higher = more unfinished).
pub fn calculate_unfinished_score(extraction: &ExtractionResult) -> f32 {
    let mut score: f32 = 0.0;

    let active_blockers = extraction.blockers.iter().filter(|b| !b.resolved).count() as f32;
    score += (active_blockers * 0.15).min(0.5);

    let incomplete_changes = extraction
        .code_changes
        .iter()
        .filter(|c| !c.completed)
        .count() as f32;
    score += (incomplete_changes * 0.1).min(0.3);

    let high_priority_handoffs = extraction
        .handoff_instructions
        .iter()
        .filter(|h| h.priority > 0.7)
        .count() as f32;
    score += (high_priority_handoffs * 0.1).min(0.2);

    let unconfirmed_decisions = extraction.decisions.iter().filter(|d| !d.confirmed).count() as f32;
    score += (unconfirmed_decisions * 0.05).min(0.2);

    let unmet_hard_reqs = extraction
        .user_requirements
        .iter()
        .filter(|r| r.hard_constraint)
        .count() as f32;
    score += (unmet_hard_reqs * 0.05).min(0.1);

    if extraction.task_state.is_none() {
        score += 0.1;
    } else if let Some(ts) = &extraction.task_state {
        if ts.next_action.is_some() {
            score += 0.1;
        }
        if !ts.blockers.is_empty() {
            score += 0.1;
        }
        if ts.verification.is_some() {
            score += 0.05;
        }
    }

    if let Some(arc) = &extraction.narrative_arc {
        let missing = [
            arc.why.is_none(),
            arc.scenario.is_none(),
            arc.vehicle.is_none(),
            arc.architecture.is_none(),
        ]
        .iter()
        .filter(|&&x| x)
        .count() as f32;
        score += (missing * 0.05).min(0.2);
    } else {
        score += 0.1;
    }

    score.min(1.0)
}

/// Calculate handoff priority (0.0-1.0, higher = more urgent).
pub fn calculate_handoff_priority(extraction: &ExtractionResult) -> f32 {
    let mut priority: f32 = 0.0;

    if let Some(top) = extraction.handoff_instructions.first() {
        priority = top.priority;
    }

    if extraction
        .blockers
        .iter()
        .any(|b| !b.resolved && b.severity > 0.7)
    {
        priority = priority.max(0.85);
    }

    if extraction.code_changes.iter().any(|c| {
        !c.completed
            && matches!(
                c.change_type,
                super::CodeChangeType::Fix
                    | super::CodeChangeType::Security
                    | super::CodeChangeType::Compliance
            )
    }) {
        priority = priority.max(0.8);
    }

    if extraction
        .user_requirements
        .iter()
        .any(|r| r.hard_constraint && r.category == super::RequirementCategory::Compliance)
    {
        priority = priority.max(0.9);
    }

    if extraction
        .handoff_instructions
        .iter()
        .any(|h| h.instruction_type == super::HandoffType::Redesign)
    {
        priority = priority.max(0.9);
    }

    priority.min(1.0)
}

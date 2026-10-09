//! Applicability of a decision's conditions against the current task, and the
//! ancestry walk that decides which ancestors stay quoted and which stay guarded.

use super::model::{Decision, Task};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Default)]
pub(super) struct Applicability {
    missing: Vec<String>,
    mismatched: BTreeMap<String, String>,
}

impl Applicability {
    pub(super) fn for_decision(task: &Task, decision: &Decision) -> Self {
        let mut result = Self::default();
        for (key, expected) in &decision.conditions {
            match task.conditions.get(key) {
                Some(actual) if actual == expected => {}
                Some(actual) => {
                    result.mismatched.insert(key.clone(), actual.clone());
                }
                None => result.missing.push(key.clone()),
            }
        }
        result
    }

    fn state(&self) -> &'static str {
        if !self.mismatched.is_empty() {
            "mismatch"
        } else if !self.missing.is_empty() {
            "unknown"
        } else {
            "applicable"
        }
    }

    pub(super) fn applicable(&self) -> bool {
        self.missing.is_empty() && self.mismatched.is_empty()
    }

    fn json(&self, expected: &BTreeMap<String, String>) -> Value {
        json!({
            "state": self.state(),
            "expected_conditions": expected,
            "missing_conditions": self.missing,
            "mismatched_actual_conditions": self.mismatched,
        })
    }
}

pub(super) fn active_item(
    decision: &Decision,
    applicability: &Applicability,
    competing: &[String],
    dependency_guards: &[String],
) -> Value {
    let mut value = json!({
        "kind": "current_decision",
        "id": decision.id,
        "project": decision.project,
        "actions": decision.actions,
        "observed_at": decision.observed_at,
        "rationale": decision.rationale,
        "lessons": decision.lessons,
        "reopen_when": decision.reopen_when,
        "sources": decision.sources,
        "applicability": applicability.json(&decision.conditions),
        "competing_ids": competing,
        "dependency_guard_ids": dependency_guards,
    });
    if applicability.applicable() && competing.is_empty() && dependency_guards.is_empty() {
        value["statement"] = Value::String(decision.statement.clone());
    }
    value
}

pub(super) fn active_dependency_item(
    decision: &Decision,
    applicability: &Applicability,
    dependency_guards: &[String],
) -> Value {
    let mut value = json!({
        "kind": "active_dependency",
        "id": decision.id,
        "project": decision.project,
        "actions": decision.actions,
        "observed_at": decision.observed_at,
        "rationale": decision.rationale,
        "lessons": decision.lessons,
        "reopen_when": decision.reopen_when,
        "sources": decision.sources,
        "applicability": applicability.json(&decision.conditions),
        "dependency_guard_ids": dependency_guards,
    });
    if applicability.applicable() && dependency_guards.is_empty() {
        value["statement"] = Value::String(decision.statement.clone());
    }
    value
}

pub(super) fn ancestor_item(decision: &Decision, relationship: &'static str) -> Value {
    json!({
        "kind": "decision_history_summary",
        "relationship": relationship,
        "id": decision.id,
        "project": decision.project,
        "actions": decision.actions,
        "observed_at": decision.observed_at,
        "rationale": decision.rationale,
        "lessons": decision.lessons,
        "reopen_when": decision.reopen_when,
        "sources": decision.sources,
        "statement_available_via_history_expand": true,
    })
}

pub(super) fn collect_ancestors(
    index: usize,
    events: &[Decision],
    positions: &HashMap<&str, usize>,
    seen: &mut BTreeSet<usize>,
    output: &mut Vec<(usize, &'static str)>,
) {
    for (relationship, refs) in [
        ("dependency", &events[index].depends_on),
        ("superseded", &events[index].supersedes),
    ] {
        for reference in refs {
            let ancestor = positions[reference.as_str()];
            if seen.insert(ancestor) {
                output.push((ancestor, relationship));
                collect_ancestors(ancestor, events, positions, seen, output);
            }
        }
    }
}

pub(super) fn supersession_ancestors(
    index: usize,
    events: &[Decision],
    positions: &HashMap<&str, usize>,
    output: &mut BTreeSet<usize>,
) {
    for reference in &events[index].supersedes {
        let ancestor = positions[reference.as_str()];
        if output.insert(ancestor) {
            supersession_ancestors(ancestor, events, positions, output);
        }
    }
}

pub(super) fn dependency_guard_ids(
    index: usize,
    task: &Task,
    events: &[Decision],
    positions: &HashMap<&str, usize>,
    superseded: &HashSet<&str>,
    competing_by_index: &BTreeMap<usize, Vec<String>>,
    memo: &mut HashMap<usize, Vec<String>>,
) -> Vec<String> {
    if let Some(cached) = memo.get(&index) {
        return cached.clone();
    }
    let mut result = BTreeSet::new();
    for reference in &events[index].depends_on {
        let dependency = positions[reference.as_str()];
        if superseded.contains(reference.as_str())
            || !Applicability::for_decision(task, &events[dependency]).applicable()
            || competing_by_index.contains_key(&dependency)
        {
            result.insert(reference.clone());
        }
        result.extend(dependency_guard_ids(
            dependency,
            task,
            events,
            positions,
            superseded,
            competing_by_index,
            memo,
        ));
    }
    let result = result.into_iter().collect::<Vec<_>>();
    memo.insert(index, result.clone());
    result
}

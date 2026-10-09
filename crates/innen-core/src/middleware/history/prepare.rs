//! The orchestrating use case: validate, select, guard, and hand the bounded
//! packet to the generic middleware.

use super::ancestry::{
    active_dependency_item, active_item, ancestor_item, collect_ancestors, dependency_guard_ids,
    supersession_ancestors, Applicability,
};
use super::model::{source_sha256, validate, Error, PreparedHistory, Request, TaskState};
use crate::conversation::grammar::tokens;
use crate::middleware::Item as MiddlewareItem;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Validate decisions, select the task's current decision history, and then
/// delegate bounded packet construction to [`crate::middleware::prepare`].
pub fn prepare(request: Request) -> Result<PreparedHistory, Error> {
    let positions = validate(&request)?;
    if request.task.state != TaskState::Active {
        let state = match request.task.state {
            TaskState::Paused => "paused",
            TaskState::Completed => "completed",
            TaskState::Active => unreachable!("active task was checked above"),
        };
        return Err(Error::InactiveTask {
            task_id: request.task.id.clone(),
            state,
        });
    }
    let history_source_sha256 = source_sha256(&request);
    let superseded = request
        .events
        .iter()
        .flat_map(|decision| decision.supersedes.iter().map(String::as_str))
        .collect::<HashSet<_>>();
    let matching = request
        .events
        .iter()
        .enumerate()
        .filter(|(_, decision)| {
            decision.project == request.task.project
                && decision.task_id == request.task.id
                && decision
                    .actions
                    .iter()
                    .any(|action| action == &request.task.action)
                && !superseded.contains(decision.id.as_str())
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    // Discover branch conflicts across every current decision in this project.
    // A later task can depend on a decision from another action, so looking at
    // task roots alone would silently accept a conflicting dependency branch.
    let current_project = request
        .events
        .iter()
        .enumerate()
        .filter(|(_, decision)| {
            decision.project == request.task.project
                && decision.task_id == request.task.id
                && !superseded.contains(decision.id.as_str())
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut supersession_ancestor_sets = BTreeMap::<usize, BTreeSet<usize>>::new();
    for &index in &current_project {
        let mut ancestors = BTreeSet::new();
        supersession_ancestors(index, &request.events, &positions, &mut ancestors);
        supersession_ancestor_sets.insert(index, ancestors);
    }
    let mut competing_by_index = BTreeMap::<usize, Vec<String>>::new();
    for &index in &current_project {
        let peers = current_project
            .iter()
            .copied()
            .filter(|other| *other != index)
            .filter(|other| {
                request.events[index]
                    .actions
                    .iter()
                    .any(|action| request.events[*other].actions.contains(action))
            })
            .filter(|other| {
                !supersession_ancestor_sets[&index].is_disjoint(&supersession_ancestor_sets[other])
            })
            .map(|other| request.events[other].id.clone())
            .collect::<Vec<_>>();
        if !peers.is_empty() {
            competing_by_index.insert(index, peers);
        }
    }
    let mut items = Vec::new();
    let mut selected_decision_ids = Vec::new();
    let mut guarded_decision_ids = Vec::new();
    let mut dependency_guard_memo = HashMap::new();
    for &index in &matching {
        let decision = &request.events[index];
        let applicability = Applicability::for_decision(&request.task, decision);
        let competing = competing_by_index.get(&index).cloned().unwrap_or_default();
        // A dependency on a superseded or guarded event is never silently
        // treated as a dependency on its replacement.
        let mut dependency_guards = dependency_guard_ids(
            index,
            &request.task,
            &request.events,
            &positions,
            &superseded,
            &competing_by_index,
            &mut dependency_guard_memo,
        );
        if let Some(peers) = competing_by_index.get(&index) {
            dependency_guards.extend(peers.clone());
            dependency_guards.sort();
            dependency_guards.dedup();
        }
        if !applicability.applicable() || !competing.is_empty() || !dependency_guards.is_empty() {
            guarded_decision_ids.push(decision.id.clone());
        }
        selected_decision_ids.push(decision.id.clone());
        items.push(MiddlewareItem {
            id: format!("history:current:{}", decision.id),
            content: active_item(decision, &applicability, &competing, &dependency_guards),
            required: true,
            priority: None,
        });
    }
    if matching.is_empty() {
        return Err(Error::NoMatchingDecision {
            project: request.task.project.clone(),
            action: request.task.action.clone(),
        });
    }
    let mut ancestors = Vec::new();
    let mut seen = matching.iter().copied().collect::<BTreeSet<_>>();
    for &index in &matching {
        collect_ancestors(
            index,
            &request.events,
            &positions,
            &mut seen,
            &mut ancestors,
        );
    }
    ancestors.sort_by_key(|(index, _)| *index);
    let mut full_history_scope = matching.iter().copied().collect::<BTreeSet<_>>();
    full_history_scope.extend(ancestors.iter().map(|(index, _)| *index));
    let full_history_events = full_history_scope
        .iter()
        .map(|index| request.events[*index].clone())
        .collect::<Vec<_>>();
    let history_before_reference_tokens = tokens(&json!({
        "stable_prefix": request.stable_prefix.clone(),
        "task": request.task.clone(),
        "events": full_history_events,
    }))
    .map_err(crate::middleware::Error::Token)?;
    for (index, relationship) in ancestors {
        let decision = &request.events[index];
        let mut dependency_guards = dependency_guard_ids(
            index,
            &request.task,
            &request.events,
            &positions,
            &superseded,
            &competing_by_index,
            &mut dependency_guard_memo,
        );
        if let Some(peers) = competing_by_index.get(&index) {
            dependency_guards.extend(peers.clone());
            dependency_guards.sort();
            dependency_guards.dedup();
        }
        let is_active_dependency =
            relationship == "dependency" && !superseded.contains(decision.id.as_str());
        items.push(MiddlewareItem {
            id: format!("history:{relationship}:{}", decision.id),
            content: if is_active_dependency {
                active_dependency_item(
                    decision,
                    &Applicability::for_decision(&request.task, decision),
                    &dependency_guards,
                )
            } else {
                ancestor_item(decision, relationship)
            },
            required: true,
            priority: None,
        });
    }
    let competing_decision_ids = matching
        .iter()
        .filter_map(|index| competing_by_index.get(index))
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let resolution = if !competing_decision_ids.is_empty() {
        "unknown_conflict"
    } else if !guarded_decision_ids.is_empty() {
        "unknown"
    } else {
        "prepared"
    };
    let prepared = crate::middleware::prepare(crate::middleware::Request {
        stable_prefix: request.stable_prefix,
        task: serde_json::to_value(&request.task).expect("history task serializes"),
        items,
        budget_tokens: request.budget_tokens,
    })?;
    Ok(PreparedHistory {
        resolution,
        history_source_sha256,
        selected_decision_ids,
        guarded_decision_ids,
        competing_decision_ids,
        history_before_reference_tokens,
        history_after_reference_tokens: prepared.after_reference_tokens,
        prepared,
    })
}

//! Evidence-bearing, task-scoped decision-history selection.
//!
//! The caller owns immutable chronological decision events.  This module only
//! validates their graph and projects the current task's effective decisions
//! into the bounded generic middleware packet.  It never turns a guarded or
//! superseded statement into an active instruction.

use super::Item as MiddlewareItem;
use crate::conversation::grammar::tokens;
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub mod store;

const MAX_DECISIONS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub stable_prefix: Value,
    pub task: Task,
    pub budget_tokens: usize,
    /// Immutable decision events in ascending canonical UTC `observed_at` order.
    pub events: Vec<Decision>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub project: String,
    pub action: String,
    #[serde(default)]
    pub state: TaskState,
    #[serde(default)]
    pub conditions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Active,
    Paused,
    Completed,
}

impl Default for TaskState {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub id: String,
    pub project: String,
    pub task_id: String,
    pub actions: Vec<String>,
    pub statement: String,
    pub rationale: String,
    pub conditions: BTreeMap<String, String>,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub lessons: Vec<String>,
    #[serde(default)]
    pub reopen_when: Vec<String>,
    /// Canonical UTC timestamp: `YYYY-MM-DDTHH:MM:SSZ`.
    pub observed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PreparedHistory {
    pub resolution: &'static str,
    pub history_source_sha256: String,
    pub selected_decision_ids: Vec<String>,
    pub guarded_decision_ids: Vec<String>,
    pub competing_decision_ids: Vec<String>,
    /// Full original events for this task's selected causal scope only.
    /// This is a reference-token comparison, not API billing.
    pub history_before_reference_tokens: usize,
    pub history_after_reference_tokens: usize,
    pub prepared: super::Prepared,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("history request has {actual} decisions, exceeding the {max} decision bound")]
    TooManyDecisions { actual: usize, max: usize },
    #[error("history task project and action must be nonempty")]
    InvalidTask,
    #[error("history task `{task_id}` is {state} and cannot prepare an active packet")]
    InactiveTask {
        task_id: String,
        state: &'static str,
    },
    #[error("no current history decision matches project `{project}` and action `{action}`")]
    NoMatchingDecision { project: String, action: String },
    #[error("history decision at index {index} has an invalid immutable shape")]
    InvalidDecision { index: usize },
    #[error("history decision `{id}` duplicates an earlier id")]
    DuplicateId { id: String },
    #[error("history decision `{id}` has invalid observed_at `{observed_at}`")]
    InvalidObservedAt { id: String, observed_at: String },
    #[error("history decision `{id}` is not in chronological order")]
    NonChronological { id: String },
    #[error("history decision `{id}` references missing or later decision `{reference}`")]
    InvalidReference { id: String, reference: String },
    #[error("history decision `{id}` has a cross-project {kind} reference `{reference}`")]
    CrossProjectReference {
        id: String,
        kind: &'static str,
        reference: String,
    },
    #[error("history decision `{id}` has a cross-task {kind} reference `{reference}`")]
    CrossTaskReference {
        id: String,
        kind: &'static str,
        reference: String,
    },
    #[error("history source hash does not match the supplied request")]
    SourceMismatch { expected: String, actual: String },
    #[error("requested history decision ids are unknown: {ids:?}")]
    UnknownDecisions { ids: Vec<String> },
    #[error(transparent)]
    Middleware(#[from] super::Error),
}

pub fn source_sha256(request: &Request) -> String {
    sha256_hex(&serde_json::to_vec(request).expect("history request serializes"))
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .any(|index| !bytes[*index].is_ascii_digit())
    {
        return false;
    }
    let number = |start| {
        std::str::from_utf8(&bytes[start..start + 2])
            .ok()?
            .parse::<u32>()
            .ok()
    };
    let year = std::str::from_utf8(&bytes[0..4])
        .ok()
        .and_then(|x| x.parse::<u32>().ok());
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        year,
        number(5),
        number(8),
        number(11),
        number(14),
        number(17),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day >= 1 && day <= max_day && hour < 24 && minute < 60 && second < 60
}

fn valid_source(source: &Source) -> bool {
    !source.id.trim().is_empty()
        && source.sha256.len() == 64
        && source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn present(value: &str) -> bool {
    !value.trim().is_empty()
}

pub(crate) fn validate(request: &Request) -> Result<HashMap<&str, usize>, Error> {
    if request.events.len() > MAX_DECISIONS {
        return Err(Error::TooManyDecisions {
            actual: request.events.len(),
            max: MAX_DECISIONS,
        });
    }
    if !present(&request.task.id)
        || !present(&request.task.project)
        || !present(&request.task.action)
    {
        return Err(Error::InvalidTask);
    }
    let mut positions = HashMap::new();
    let mut previous_time: Option<&str> = None;
    for (index, decision) in request.events.iter().enumerate() {
        if !present(&decision.id)
            || !present(&decision.project)
            || !present(&decision.task_id)
            || decision.actions.is_empty()
            || decision.actions.iter().any(|action| !present(action))
            || !present(&decision.statement)
            || !present(&decision.rationale)
            || decision.sources.is_empty()
            || decision.sources.iter().any(|source| !valid_source(source))
        {
            return Err(Error::InvalidDecision { index });
        }
        if !valid_timestamp(&decision.observed_at) {
            return Err(Error::InvalidObservedAt {
                id: decision.id.clone(),
                observed_at: decision.observed_at.clone(),
            });
        }
        if previous_time.is_some_and(|previous| previous > decision.observed_at.as_str()) {
            return Err(Error::NonChronological {
                id: decision.id.clone(),
            });
        }
        previous_time = Some(&decision.observed_at);
        if positions.insert(decision.id.as_str(), index).is_some() {
            return Err(Error::DuplicateId {
                id: decision.id.clone(),
            });
        }
    }
    for (index, decision) in request.events.iter().enumerate() {
        for (kind, refs) in [
            ("supersession", &decision.supersedes),
            ("dependency", &decision.depends_on),
        ] {
            let mut unique = HashSet::new();
            for reference in refs {
                if !present(reference) || !unique.insert(reference.as_str()) {
                    return Err(Error::InvalidDecision { index });
                }
                let Some(&other_index) = positions.get(reference.as_str()) else {
                    return Err(Error::InvalidReference {
                        id: decision.id.clone(),
                        reference: reference.clone(),
                    });
                };
                if other_index >= index {
                    return Err(Error::InvalidReference {
                        id: decision.id.clone(),
                        reference: reference.clone(),
                    });
                }
                if request.events[other_index].project != decision.project {
                    return Err(Error::CrossProjectReference {
                        id: decision.id.clone(),
                        kind,
                        reference: reference.clone(),
                    });
                }
                if request.events[other_index].task_id != decision.task_id {
                    return Err(Error::CrossTaskReference {
                        id: decision.id.clone(),
                        kind,
                        reference: reference.clone(),
                    });
                }
            }
        }
    }
    Ok(positions)
}

#[derive(Default)]
struct Applicability {
    missing: Vec<String>,
    mismatched: BTreeMap<String, String>,
}

impl Applicability {
    fn for_decision(task: &Task, decision: &Decision) -> Self {
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

    fn applicable(&self) -> bool {
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

fn active_item(
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

fn active_dependency_item(
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

fn ancestor_item(decision: &Decision, relationship: &'static str) -> Value {
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

fn collect_ancestors(
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

fn supersession_ancestors(
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

fn dependency_guard_ids(
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

/// Validate decisions, select the task's current decision history, and then
/// delegate bounded packet construction to [`super::prepare`].
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
    .map_err(super::Error::Token)?;
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
    let prepared = super::prepare(super::Request {
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

/// Return exact immutable decision envelopes only when the whole history input
/// still matches `expected_source_sha256`.
pub fn expand(
    request: Request,
    expected_source_sha256: &str,
    ids: &[String],
) -> Result<Value, Error> {
    validate(&request)?;
    let actual = source_sha256(&request);
    if expected_source_sha256 != actual {
        return Err(Error::SourceMismatch {
            expected: expected_source_sha256.to_owned(),
            actual,
        });
    }
    let known = request
        .events
        .iter()
        .map(|decision| decision.id.as_str())
        .collect::<BTreeSet<_>>();
    let unknown = ids
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(Error::UnknownDecisions { ids: unknown });
    }
    let wanted = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let events = request
        .events
        .into_iter()
        .filter(|decision| wanted.contains(decision.id.as_str()))
        .collect::<Vec<_>>();
    Ok(json!({
        "resolution": "expanded",
        "history_source_sha256": actual,
        "events": events,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(id: &str) -> Value {
        json!({"id":id,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})
    }

    fn decision(id: &str, observed_at: &str, action: &str) -> Value {
        json!({
            "id":id,"project":"innen","task_id":"maintenance","actions":[action],"statement":format!("{id} statement"),
            "rationale":format!("{id} rationale"),"conditions":{},"sources":[source(id)],
            "supersedes":[],"depends_on":[],"lessons":[format!("{id} lesson")],
            "reopen_when":[],"observed_at":observed_at
        })
    }

    fn request(events: Vec<Value>) -> Request {
        serde_json::from_value(json!({
            "stable_prefix":{"system":"stable"},
            "task":{"id":"maintenance","project":"innen","action":"clean","conditions":{"os":"macos"}},
            "budget_tokens":10000,"events":events
        }))
        .unwrap()
    }

    #[test]
    fn keeps_current_statement_and_ancestor_lessons_without_old_statement() {
        let mut old = decision("old", "2026-09-08T09:00:00Z", "clean");
        old["conditions"] = json!({"os":"macos"});
        let mut current = decision("current", "2026-09-08T10:00:00Z", "clean");
        current["supersedes"] = json!(["old"]);
        let result = prepare(request(vec![old, current])).unwrap();
        assert_eq!(result.resolution, "prepared");
        assert_eq!(result.selected_decision_ids, ["current"]);
        let rendered = result.prepared.packet.to_string();
        assert!(rendered.contains("current statement"));
        assert!(!rendered.contains("old statement"));
        assert!(rendered.contains("old lesson"));
    }

    #[test]
    fn guarded_replacement_does_not_fallback_to_superseded_statement() {
        let old = decision("old", "2026-09-08T09:00:00Z", "clean");
        let mut current = decision("current", "2026-09-08T10:00:00Z", "clean");
        current["supersedes"] = json!(["old"]);
        current["conditions"] = json!({"os":"linux"});
        let result = prepare(request(vec![old, current])).unwrap();
        assert_eq!(result.resolution, "unknown");
        assert_eq!(result.guarded_decision_ids, ["current"]);
        let rendered = result.prepared.packet.to_string();
        assert!(!rendered.contains("current statement"));
        assert!(!rendered.contains("old statement"));
        assert!(rendered.contains("mismatch"));
    }

    #[test]
    fn branching_replacements_are_explicit_conflict() {
        let old = decision("old", "2026-09-08T09:00:00Z", "clean");
        let mut left = decision("left", "2026-09-08T10:00:00Z", "clean");
        left["supersedes"] = json!(["old"]);
        let mut right = decision("right", "2026-09-08T11:00:00Z", "clean");
        right["supersedes"] = json!(["old"]);
        let result = prepare(request(vec![old, left, right])).unwrap();
        assert_eq!(result.resolution, "unknown_conflict");
        assert_eq!(result.competing_decision_ids, ["left", "right"]);
        assert!(!result
            .prepared
            .packet
            .to_string()
            .contains("left statement"));
    }

    #[test]
    fn dependency_on_superseded_event_is_stale_not_reactivated() {
        let old = decision("old", "2026-09-08T09:00:00Z", "clean");
        let mut replacement = decision("replacement", "2026-09-08T10:00:00Z", "clean");
        replacement["supersedes"] = json!(["old"]);
        let mut dependent = decision("dependent", "2026-09-08T11:00:00Z", "clean");
        dependent["depends_on"] = json!(["old"]);
        let result = prepare(request(vec![old, replacement, dependent])).unwrap();
        assert_eq!(result.resolution, "unknown");
        assert!(result
            .guarded_decision_ids
            .contains(&"dependent".to_owned()));
        let rendered = result.prepared.packet.to_string();
        assert!(rendered.contains("dependency_guard_ids"));
        assert!(!rendered.contains("dependent statement"));
        assert!(!rendered.contains("old statement"));
    }

    #[test]
    fn applicable_current_dependency_keeps_its_statement() {
        let dependency = decision("dependency", "2026-09-08T09:00:00Z", "configure");
        let mut current = decision("current", "2026-09-08T10:00:00Z", "clean");
        current["depends_on"] = json!(["dependency"]);
        let result = prepare(request(vec![dependency, current])).unwrap();
        let rendered = result.prepared.packet.to_string();
        assert!(rendered.contains("dependency statement"));
        assert!(rendered.contains("active_dependency"));
    }

    #[test]
    fn transitive_supersession_branches_are_explicit_conflict() {
        let old = decision("old", "2026-09-08T09:00:00Z", "clean");
        let mut left = decision("left", "2026-09-08T10:00:00Z", "clean");
        left["supersedes"] = json!(["old"]);
        let mut left_latest = decision("left-latest", "2026-09-08T11:00:00Z", "clean");
        left_latest["supersedes"] = json!(["left"]);
        let mut right = decision("right", "2026-09-08T12:00:00Z", "clean");
        right["supersedes"] = json!(["old"]);
        let result = prepare(request(vec![old, left, left_latest, right])).unwrap();
        assert_eq!(result.resolution, "unknown_conflict");
        assert_eq!(result.competing_decision_ids, ["left-latest", "right"]);
    }

    #[test]
    fn cross_action_dependency_on_conflicting_branch_is_guarded() {
        let old = decision("old", "2026-09-08T09:00:00Z", "verify");
        let mut left = decision("left", "2026-09-08T10:00:00Z", "verify");
        left["supersedes"] = json!(["old"]);
        let mut right = decision("right", "2026-09-08T11:00:00Z", "verify");
        right["supersedes"] = json!(["old"]);
        let mut release = decision("release", "2026-09-08T12:00:00Z", "clean");
        release["depends_on"] = json!(["left"]);
        let result = prepare(request(vec![old, left, right, release])).unwrap();
        assert_eq!(result.resolution, "unknown");
        assert!(result.guarded_decision_ids.contains(&"release".to_owned()));
        assert!(!result
            .prepared
            .packet
            .to_string()
            .contains("release statement"));
    }

    #[test]
    fn no_match_is_explicit_unknown() {
        assert!(matches!(
            prepare(request(vec![decision(
                "other",
                "2026-09-08T09:00:00Z",
                "publish",
            )])),
            Err(Error::NoMatchingDecision { .. })
        ));
    }

    #[test]
    fn rejects_noncanonical_time_missing_provenance_and_cross_project_reference() {
        let mut bad_time = decision("a", "2026-9-08T09:00:00Z", "clean");
        assert!(matches!(
            prepare(request(vec![bad_time.clone()])),
            Err(Error::InvalidObservedAt { .. })
        ));
        bad_time["observed_at"] = json!("2026-09-08T09:00:00Z");
        bad_time["sources"] = json!([]);
        assert!(matches!(
            prepare(request(vec![bad_time])),
            Err(Error::InvalidDecision { .. })
        ));
        let first = decision("a", "2026-09-08T09:00:00Z", "clean");
        let mut cross = decision("b", "2026-09-08T10:00:00Z", "clean");
        cross["project"] = json!("other");
        cross["depends_on"] = json!(["a"]);
        assert!(matches!(
            prepare(request(vec![first, cross])),
            Err(Error::CrossProjectReference { .. })
        ));
        let first = decision("a", "2026-09-08T09:00:00Z", "clean");
        let mut cross_task = decision("b", "2026-09-08T10:00:00Z", "clean");
        cross_task["task_id"] = json!("other-task");
        cross_task["depends_on"] = json!(["a"]);
        assert!(matches!(
            prepare(request(vec![first, cross_task])),
            Err(Error::CrossTaskReference { .. })
        ));
    }

    #[test]
    fn paused_task_cannot_prepare_and_exact_task_identity_is_required() {
        let event = decision("one", "2026-09-08T09:00:00Z", "clean");
        let mut paused = request(vec![event.clone()]);
        paused.task.state = TaskState::Paused;
        assert!(matches!(
            prepare(paused),
            Err(Error::InactiveTask {
                state: "paused",
                ..
            })
        ));
        let mut wrong_task = request(vec![event]);
        wrong_task.task.id = "different-task".into();
        assert!(matches!(
            prepare(wrong_task),
            Err(Error::NoMatchingDecision { .. })
        ));
    }

    #[test]
    fn expansion_returns_exact_envelopes_only_for_matching_hash() {
        let input = request(vec![decision("one", "2026-09-08T09:00:00Z", "clean")]);
        let hash = source_sha256(&input);
        let expanded = expand(input, &hash, &["one".to_owned()]).unwrap();
        assert_eq!(expanded["events"][0]["statement"], "one statement");
    }
}

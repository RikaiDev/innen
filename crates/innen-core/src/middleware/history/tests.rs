//! Behaviour tests for task-scoped decision-history selection.

use super::*;
use serde_json::{json, Value};

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

use assert_cmd::Command;
use innen_core::journal::Journal;
use serde_json::{json, Value};

fn append(root: &std::path::Path, op: &str, value: Value) {
    Journal::open(root).unwrap().append(op, &value).unwrap();
}
fn run(root: &std::path::Path, args: &[&str]) -> Value {
    let result = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .arg("project")
        .args(args)
        .assert()
        .success();
    serde_json::from_slice(&result.get_output().stdout).unwrap()
}
fn fixture(root: &std::path::Path) {
    for id in ["project:one", "project:empty"] {
        append(
            root,
            "node.upsert",
            json!({"id":id,"type":"Project","label":id}),
        );
    }
    for (id, status) in [
        ("task:open", "open"),
        ("task:done", "open"),
        ("task:unknown", ""),
        ("task:partial", "content-complete-layout-blocked"),
        ("task:retracted", "open"),
    ] {
        append(
            root,
            "node.upsert",
            json!({"id":id,"type":"Task","label":format!("工作 {id}"),"status":status}),
        );
        append(
            root,
            "edge.assert",
            json!({"from":id,"to":"project:one","type":"BELONGS_TO","evidence":"original meeting"}),
        );
    }
    // Completion may be a status-only update; retain the original label.
    append(
        root,
        "node.upsert",
        json!({"id":"task:done","status":"complete"}),
    );
    append(
        root,
        "node.upsert",
        json!({"id":"task:open","next_action":"Run paired validation","blockers":["Missing gold labels"]}),
    );
    // Duplicate edges cannot produce duplicate task rows.
    append(
        root,
        "edge.assert",
        json!({"from":"task:open","to":"project:one","type":"BELONGS_TO"}),
    );
    append(
        root,
        "edge.retract",
        json!({"from":"task:retracted","to":"project:one","type":"BELONGS_TO"}),
    );
}

#[test]
fn brief_filters_only_explicit_terminal_status_and_keeps_unknown() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["one"]);
    assert_eq!(out["total"], 3);
    assert_eq!(out["excluded_terminal"], 1);
    let rows = out["rows"].as_array().unwrap();
    assert_eq!(rows[0][0], "task:open");
    assert_eq!(rows[0][5], "Run paired validation");
    assert_eq!(rows[0][6], json!(["Missing gold labels"]));
    assert_eq!(rows[1][2], "content-complete-layout-blocked");
    assert_eq!(rows[2][2], "unknown");
    assert!(rows.iter().all(|r| r[0] != "task:done"));
}

#[test]
fn evidence_preserves_completion_history_and_edge_provenance() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["one", "--view", "evidence"]);
    let task = out["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "task:done")
        .unwrap();
    assert_eq!(task["status"], "complete");
    assert_eq!(task["label"], "工作 task:done");
    let history = task["history"].as_array().unwrap();
    assert!(history.iter().any(|e| e["payload"]["status"] == "open"));
    assert!(history
        .iter()
        .any(|e| e["payload"]["evidence"] == "original meeting"));
    assert!(history
        .iter()
        .all(|e| e["line"].as_u64().unwrap() > 0 && e["event"].as_str().is_some()));
    let brief = run(dir.path(), &["one"]);
    for row in brief["rows"].as_array().unwrap() {
        let detail = out["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == row[0])
            .unwrap();
        for (index, field) in brief["fields"].as_array().unwrap().iter().enumerate() {
            assert_eq!(row[index], detail[field.as_str().unwrap()]);
        }
    }
}

#[test]
fn expired_membership_is_not_current_and_reopened_work_returns() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    append(
        dir.path(),
        "node.upsert",
        json!({"id":"task:done","status":"open"}),
    );
    append(
        dir.path(),
        "node.upsert",
        json!({"id":"task:expired","type":"Task","status":"open"}),
    );
    append(
        dir.path(),
        "edge.assert",
        json!({"from":"task:expired","to":"project:one","type":"BELONGS_TO","valid_from":"2020-01-01T00:00:00Z","valid_until":"2020-01-02T00:00:00Z"}),
    );
    let out = run(dir.path(), &["one"]);
    assert_eq!(out["excluded_terminal"], 0);
    let rows = out["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r[0] == "task:done"));
    assert!(rows.iter().all(|r| r[0] != "task:expired"));
}

#[test]
fn portfolio_retains_unassigned_tasks_empty_projects_and_pagination() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["--limit", "2"]);
    assert_eq!(out["total"], 4);
    assert!(out["projects"].get("project:empty").is_some());
    assert_eq!(out["next_offset"], 2);
    let next = run(dir.path(), &["--offset", "2", "--limit", "2"]);
    assert_eq!(next["rows"].as_array().unwrap().len(), 2);
    assert!(next["next_offset"].is_null());
    let empty = run(dir.path(), &["empty"]);
    assert_eq!(empty["total"], 0);
    assert!(empty["scope"].as_str().unwrap().contains("recorded"));
}

#[test]
fn missing_kb_fails_without_creating_state_and_full_view_remains_available() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(dir.path())
        .arg("project")
        .assert()
        .failure();
    assert!(!dir.path().join(".innen").exists());
    fixture(dir.path());
    let legacy = run(dir.path(), &["one", "--view", "full"]);
    assert!(legacy["render"]
        .as_str()
        .unwrap()
        .contains("## Experiments"));
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(dir.path())
        .args(["--format", "human", "project", "one"])
        .assert()
        .success();
}

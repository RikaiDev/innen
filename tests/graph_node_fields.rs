//! `innen graph node` writes the optional status/summary/date fields a task needs
//! to appear and retire in `innen project --view brief`.
//!
//! Before these flags existed, every CLI-created task materialised without a
//! status, rendered as `[unknown]`, and could never be retired: `is_terminal`
//! only excludes complete|completed|done|closed|cancelled|canceled|superseded,
//! none of which the CLI could write. See issue #8.
use assert_cmd::Command;
use serde_json::Value;

fn node(root: &std::path::Path, args: &[&str]) -> Value {
    let out = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args(["graph", "node"])
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .to_vec();
    serde_json::from_slice(&out).unwrap()
}

fn relate(root: &std::path::Path, from: &str, to: &str) {
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args([
            "graph",
            "relate",
            "--from",
            from,
            "--edge",
            "BELONGS_TO",
            "--to",
            to,
        ])
        .assert()
        .success();
}

/// `--view brief` reports rows positionally, not as a `tasks` array: fields are
/// `["id","projects","status","label","updated","next_action","blockers"]`.
/// Index 0 is the node id, 2 the status; terminal rows are counted in
/// `excluded_terminal` and omitted from `rows`.
fn project(root: &std::path::Path) -> Value {
    let out = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args(["project", "p", "--view", "brief"])
        .assert()
        .success()
        .get_output()
        .stdout
        .to_vec();
    serde_json::from_slice(&out).unwrap()
}

/// The project node itself has to exist or resolution fails before tasks are read.
fn seed_project(root: &std::path::Path) {
    node(
        root,
        &["--id", "project:p", "--kind", "Project", "--label", "P"],
    );
}

#[test]
fn status_summary_and_date_reach_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    node(
        root,
        &[
            "--id",
            "task:p:a",
            "--kind",
            "task",
            "--label",
            "A",
            "--status",
            "open",
            "--summary",
            "what this blocks",
            "--date",
            "2026-10-05",
        ],
    );

    let stored = std::fs::read_to_string(root.join(".innen/journal.jsonl")).unwrap();
    let line: Value = serde_json::from_str(stored.lines().last().unwrap()).unwrap();
    let payload = &line["payload"];
    assert_eq!(payload["status"], "open");
    assert_eq!(payload["summary"], "what this blocks");
    assert_eq!(payload["date"], "2026-10-05");
    // The pre-existing fields must be untouched.
    assert_eq!(payload["id"], "task:p:a");
    assert_eq!(payload["label"], "A");
    assert_eq!(payload["type"], "task");
}

#[test]
fn omitting_the_flags_keeps_the_payload_minimal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    node(
        root,
        &["--id", "task:p:a", "--kind", "task", "--label", "A"],
    );

    let stored = std::fs::read_to_string(root.join(".innen/journal.jsonl")).unwrap();
    let line: Value = serde_json::from_str(stored.lines().last().unwrap()).unwrap();
    let payload = &line["payload"];
    assert!(payload.get("status").is_none());
    assert!(payload.get("summary").is_none());
    assert!(payload.get("date").is_none());
}

/// The reason the flags exist: a task carrying `open` shows its real status, and
/// re-upserting it as `complete` retires it from the brief view.
#[test]
fn a_task_can_be_shown_then_retired_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_project(root);
    // The edge is adjacency-validated, so the task must exist before it can
    // belong to the project.
    node(
        root,
        &[
            "--id", "task:p:a", "--kind", "task", "--label", "A", "--status", "open",
        ],
    );
    relate(root, "task:p:a", "project:p");

    let brief = project(root);
    let rows = brief["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], "task:p:a");
    assert_eq!(rows[0][2], "open");
    assert_eq!(brief["excluded_terminal"], 0);

    node(
        root,
        &[
            "--id", "task:p:a", "--kind", "task", "--label", "A", "--status", "complete",
        ],
    );

    let brief = project(root);
    assert_eq!(brief["rows"].as_array().unwrap().len(), 0);
    assert_eq!(brief["excluded_terminal"], 1);
}

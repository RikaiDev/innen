//! `project --view brief` must not claim status is unverified when every listed
//! row carries one.
//!
//! The notice used to be appended unconditionally, so a project whose tasks all
//! had a real status still ended with "Unknown status is unverified". See #8.
use assert_cmd::Command;
use serde_json::json;

use innen_core::journal::Journal;

fn append(root: &std::path::Path, op: &str, value: serde_json::Value) {
    Journal::open(root).unwrap().append(op, &value).unwrap();
}

fn node(root: &std::path::Path, args: &[&str]) {
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args(["graph", "node"])
        .args(args)
        .assert()
        .success();
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

fn brief(root: &std::path::Path, project: &str) -> String {
    let out = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args(["project", project, "--view", "brief", "--format", "human"])
        .assert()
        .success()
        .get_output()
        .stdout
        .to_vec();
    String::from_utf8(out).unwrap()
}

fn seed(root: &std::path::Path) {
    append(
        root,
        "node.upsert",
        json!({"id":"project:p","type":"Project","label":"P"}),
    );
}

#[test]
fn no_notice_when_every_row_has_a_status() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(root);
    node(
        root,
        &[
            "--id", "task:p:a", "--kind", "task", "--label", "A", "--status", "open",
        ],
    );
    relate(root, "task:p:a", "project:p");

    let out = brief(root, "p");
    assert!(out.contains("[open]"), "row should show its status: {out}");
    assert!(
        !out.contains("no recorded status"),
        "no notice expected when all rows have a status: {out}"
    );
}

#[test]
fn notice_counts_only_the_rows_missing_a_status() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(root);
    node(
        root,
        &[
            "--id",
            "task:p:with",
            "--kind",
            "task",
            "--label",
            "with",
            "--status",
            "open",
        ],
    );
    relate(root, "task:p:with", "project:p");
    // No --status: the CLI cannot write one, which is the case the notice is for.
    node(
        root,
        &[
            "--id",
            "task:p:without",
            "--kind",
            "task",
            "--label",
            "without",
        ],
    );
    relate(root, "task:p:without", "project:p");

    let out = brief(root, "p");
    assert!(out.contains("Recorded tasks: 2"), "{out}");
    assert!(
        out.contains("1 row(s) have no recorded status"),
        "notice should count exactly one row: {out}"
    );
}

#[test]
fn terminal_exclusion_is_still_reported_when_nothing_is_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(root);
    node(
        root,
        &[
            "--id",
            "task:p:done",
            "--kind",
            "task",
            "--label",
            "done",
            "--status",
            "complete",
        ],
    );
    relate(root, "task:p:done", "project:p");

    let out = brief(root, "p");
    assert!(
        out.contains("Recorded tasks: 0 (terminal excluded: 1)"),
        "{out}"
    );
    assert!(!out.contains("no recorded status"), "{out}");
}

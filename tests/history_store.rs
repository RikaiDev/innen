use assert_cmd::Command;
use innen_core::ids::sha256_hex;
use serde_json::{json, Value};
use std::fs;
use std::thread;
use tempfile::TempDir;

fn envelope(state: &str, action: &str) -> Value {
    json!({
        "stable_prefix":{"policy":"fixture"},
        "task":{"id":"task:one","project":"project:one","action":action,"state":state,
            "conditions":{"date":"2026-09-08"}},
        "budget_tokens":4096,
        "events":[{
            "id":"decision:one","project":"project:one","task_id":"task:one","actions":["maintain"],
            "statement":"Retain immutable evidence","rationale":"fixture evidence",
            "conditions":{},"sources":[{"id":"source:one","sha256":sha256_hex(b"fixture")}],
            "supersedes":[],"depends_on":[],"lessons":[],"reopen_when":[],
            "observed_at":"2026-09-08T00:00:00Z"
        }]
    })
}

fn save(store: &TempDir, input: &Value) -> std::process::Output {
    Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-save", "--file", "-", "--store"])
        .arg(store.path())
        .write_stdin(input.to_string())
        .output()
        .unwrap()
}

fn load(store: &TempDir, project: &str, task: &str) -> std::process::Output {
    Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-load", "--store"])
        .arg(store.path())
        .args(["--project", project, "--task", task])
        .output()
        .unwrap()
}

fn save_at(store: std::path::PathBuf, input: Value) -> std::process::Output {
    Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-save", "--file", "-", "--store"])
        .arg(store)
        .write_stdin(input.to_string())
        .output()
        .unwrap()
}

#[test]
fn saves_and_loads_latest_paused_envelope_across_dates() {
    let store = tempfile::tempdir().unwrap();
    let active = envelope("active", "maintain");
    assert!(save(&store, &active).status.success());
    let mut paused = envelope("paused", "review");
    paused["task"]["conditions"]["date"] = json!("2026-09-09");
    assert!(save(&store, &paused).status.success());
    let output = load(&store, "project:one", "task:one");
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        paused
    );
}

#[test]
fn rejects_decision_rewrite_and_retains_old_snapshot_bytes() {
    let store = tempfile::tempdir().unwrap();
    let original = envelope("active", "maintain");
    assert!(save(&store, &original).status.success());
    let mut rewritten = original.clone();
    rewritten["events"][0]["statement"] = json!("rewritten evidence");
    let rejected = save(&store, &rewritten);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stdout).contains("rewrites prior immutable decision"));
    let loaded = load(&store, "project:one", "task:one");
    assert_eq!(
        serde_json::from_slice::<Value>(&loaded.stdout).unwrap(),
        original
    );
    let mut removed = original.clone();
    removed["events"] = json!([]);
    let removed = save(&store, &removed);
    assert_eq!(removed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed.stdout).contains("removes prior immutable decision"));
}

#[test]
fn isolates_tasks_and_rejects_cross_task_events_and_malformed_input() {
    let store = tempfile::tempdir().unwrap();
    let first = envelope("completed", "maintain");
    assert!(save(&store, &first).status.success());
    let mut second = envelope("active", "maintain");
    second["task"]["id"] = json!("task:two");
    second["events"][0]["task_id"] = json!("task:two");
    assert!(save(&store, &second).status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&load(&store, "project:one", "task:one").stdout).unwrap(),
        first
    );
    let mut polluted = envelope("active", "maintain");
    let mut foreign = polluted["events"][0].clone();
    foreign["id"] = json!("decision:foreign");
    foreign["project"] = json!("project:foreign");
    foreign["task_id"] = json!("task:foreign");
    foreign["observed_at"] = json!("2026-09-08T00:00:01Z");
    polluted["events"].as_array_mut().unwrap().push(foreign);
    let pollution = save(&store, &polluted);
    assert_eq!(pollution.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&pollution.stdout).contains("outside project"));
    let malformed = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-save", "--file", "-", "--store"])
        .arg(store.path())
        .write_stdin("not json")
        .output()
        .unwrap();
    assert_eq!(malformed.status.code(), Some(2));
}

#[test]
fn concurrent_state_saves_keep_all_index_entries() {
    let store = tempfile::tempdir().unwrap();
    assert!(save(&store, &envelope("active", "maintain"))
        .status
        .success());
    let path = store.path().to_path_buf();
    let paused = envelope("paused", "maintain");
    let completed = envelope("completed", "maintain");
    let first = thread::spawn({
        let path = path.clone();
        move || save_at(path, paused)
    });
    let second = thread::spawn(move || save_at(path, completed));
    assert!(first.join().unwrap().status.success());
    assert!(second.join().unwrap().status.success());
    let key = sha256_hex(&serde_json::to_vec(&("project:one", "task:one")).unwrap());
    let index = fs::read_to_string(store.path().join(key).join("index.jsonl")).unwrap();
    assert_eq!(
        index.lines().count(),
        3,
        "each successful save has an index entry"
    );
}

#[test]
fn torn_index_and_nul_separated_task_identities_fail_closed_or_isolate() {
    let store = tempfile::tempdir().unwrap();
    assert!(save(&store, &envelope("active", "maintain"))
        .status
        .success());
    let key = sha256_hex(&serde_json::to_vec(&("project:one", "task:one")).unwrap());
    let index = store.path().join(key).join("index.jsonl");
    fs::write(&index, b"{\"history_source_sha256\":\"torn\"").unwrap();
    assert_eq!(
        load(&store, "project:one", "task:one").status.code(),
        Some(2)
    );

    let mut first = envelope("active", "maintain");
    first["task"]["project"] = json!("a\u{0}b");
    first["task"]["id"] = json!("c");
    first["events"][0]["project"] = json!("a\u{0}b");
    first["events"][0]["task_id"] = json!("c");
    let mut second = envelope("active", "maintain");
    second["task"]["project"] = json!("a");
    second["task"]["id"] = json!("b\u{0}c");
    second["events"][0]["project"] = json!("a");
    second["events"][0]["task_id"] = json!("b\u{0}c");
    let separate = tempfile::tempdir().unwrap();
    let first_request = serde_json::from_value(first.clone()).unwrap();
    let second_request = serde_json::from_value(second.clone()).unwrap();
    innen_core::middleware::history::store::save(separate.path(), first_request).unwrap();
    innen_core::middleware::history::store::save(separate.path(), second_request).unwrap();
    assert_eq!(
        serde_json::to_value(
            innen_core::middleware::history::store::load(separate.path(), "a\u{0}b", "c").unwrap()
        )
        .unwrap(),
        first
    );
    assert_eq!(
        serde_json::to_value(
            innen_core::middleware::history::store::load(separate.path(), "a", "b\u{0}c").unwrap()
        )
        .unwrap(),
        second
    );
}

#[test]
fn hook_reports_expandable_immutable_store_snapshot() {
    let store = tempfile::tempdir().unwrap();
    let request = envelope("active", "maintain");
    assert!(save(&store, &request).status.success());
    let cwd = tempfile::tempdir().unwrap();
    let event = json!({"hook_event_name":"SessionStart","session_id":"fixture-session","source":"startup","cwd":cwd.path()});
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-hook", "--store"])
        .arg(store.path())
        .args(["--project", "project:one", "--task", "task:one", "--cwd"])
        .arg(cwd.path())
        .write_stdin(event.to_string())
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let context: Value = serde_json::from_str(
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(context["resolution"], "prepared");
    assert!(context["packet"].to_string().contains("decision:one"));
    let source_hash = context["history_source_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let snapshot = context["history_file"].as_str().unwrap().to_owned();
    assert!(save(&store, &envelope("paused", "review")).status.success());
    let expanded = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "middleware",
            "history-expand",
            "--file",
            &snapshot,
            "--expect-source-sha256",
            &source_hash,
            "--ids",
            "decision:one",
        ])
        .output()
        .unwrap();
    assert!(expanded.status.success());
}

#[test]
fn direct_store_api_rejects_oversized_snapshot_before_writing() {
    let store = tempfile::tempdir().unwrap();
    let mut oversized = envelope("active", "maintain");
    oversized["events"][0]["statement"] = json!("x".repeat(16 * 1024 * 1024));
    let request = serde_json::from_value(oversized).unwrap();
    let error = innen_core::middleware::history::store::save(store.path(), request).unwrap_err();
    assert!(error.to_string().contains("snapshot exceeds"));
    assert!(fs::read_dir(store.path()).unwrap().next().is_none());
}

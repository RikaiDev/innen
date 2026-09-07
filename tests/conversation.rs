//! Source adapter behavior through the real CLI; synthetic data only.
use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::Path;

const UUID: &str = "e3a92b35-4931-427b-9adf-1baa29318ca6";

fn fixture(root: &Path, text: &str) {
    let directory = root.join(UUID).join(".system_generated/logs");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("transcript.jsonl"), text).unwrap();
}

fn command(root: &Path) -> Command {
    let mut command = Command::cargo_bin("innen").unwrap();
    command
        .args(["read", UUID, "--source", "agy", "--source-root"])
        .arg(root);
    command
}

#[test]
fn conversation_batch_lines_preserves_order_and_rejects_missing_rows() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path(),"{\"type\":\"USER_INPUT\",\"content\":\"keep failure\"}\n\n{\"type\":\"GENERIC\",\"content\":\"FAIL: not approved\"}\n");
    let output = command(tmp.path())
        .args(["--lines", "3,1,3", "--compact"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let packed: Value = serde_json::from_slice(&output).unwrap();
    let page = innen_core::conversation::compact::decode(&packed).unwrap();
    assert_eq!(page["view"], "events");
    assert_eq!(page["records"][0]["line"], 1);
    assert_eq!(page["records"][1]["line"], 3);
    assert_eq!(page["records"][1]["event"]["content"], "FAIL: not approved");
    assert_eq!(page["records"].as_array().unwrap().len(), 2);
    assert!(page["next_offset"].is_null());
    for lines in ["0", "2", "9"] {
        command(tmp.path())
            .args(["--lines", lines])
            .assert()
            .failure();
    }
}

#[test]
fn conversation_paginates_dialogue_without_losing_corrections_or_source_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let rows = [
        json!({"step_index":0,"type":"USER_INPUT","content":"<USER_REQUEST>請查架構</USER_REQUEST><META>noise</META>"}),
        json!({"step_index":1,"type":"PLANNER_RESPONSE","content":"Originally Vite"}),
        json!({"step_index":3,"type":"GENERIC","content":"tool evidence"}),
        json!({"step_index":7,"type":"PLANNER_RESPONSE","thinking":"internal","content":"Correction: Next.js","truncated_fields":["content"]}),
    ];
    let text = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fixture(tmp.path(), &text);
    let output = command(tmp.path())
        .args(["--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let page: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(page["records"][0]["event"]["content"], "請查架構");
    assert_eq!(page["next_offset"], 3);
    let output = command(tmp.path())
        .args(["--offset", "3", "--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let page: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        page["records"][0]["event"]["content"],
        "Correction: Next.js"
    );
    assert_eq!(page["records"][0]["line"], 4);
    assert!(page["records"][0]["event"].get("thinking").is_none());
    assert_eq!(page["warnings"].as_array().unwrap().len(), 1);
    assert!(page["next_offset"].is_null());
    assert!(!tmp.path().join(".innen").exists());
    assert_eq!(
        std::fs::read_to_string(
            tmp.path()
                .join(UUID)
                .join(".system_generated/logs/transcript.jsonl")
        )
        .unwrap(),
        text
    );
}

#[test]
fn conversation_events_preserve_tool_payloads_and_unknown_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let event = json!({"type":"GENERIC","step_index":9,"content":"exit 1","extra":{"exit_code":1}});
    fixture(tmp.path(), &event.to_string());
    let output = command(tmp.path())
        .args(["--view", "events"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let page: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(page["records"][0]["event"], event);
}

#[test]
fn conversation_errors_are_distinct_and_do_not_echo_corrupt_source() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = command(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8(absent)
        .unwrap()
        .contains("conversation not found"));
    fixture(tmp.path(), "{invalid PRIVATE_PAYLOAD");
    let bad = command(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let error = String::from_utf8(bad).unwrap();
    assert!(error.contains("invalid transcript JSON"));
    assert!(error.contains("transcript.jsonl:1"));
    assert!(!error.contains("PRIVATE_PAYLOAD"));
    let invalid = Command::cargo_bin("innen")
        .unwrap()
        .args(["read", "../../etc/passwd"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8(invalid).unwrap().contains("invalid UUID"));
    command(tmp.path())
        .args(["--limit", "0"])
        .assert()
        .failure();
}

#[test]
fn conversation_source_resolution_rejects_ambiguity() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["first", "second"] {
        let folder = root.path().join(dir);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(format!("{UUID}.jsonl")),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}",
        )
        .unwrap();
    }
    let error =
        innen_core::conversation::read(Some(root.path()), "claude", UUID, "dialogue", 0, 20)
            .unwrap_err();
    assert!(error.to_string().contains("ambiguous conversation"));
    let page = innen_core::conversation::read(
        Some(&root.path().join("first")),
        "claude",
        &UUID.to_uppercase(),
        "dialogue",
        0,
        20,
    )
    .unwrap();
    assert_eq!(page.session_id, UUID);
    assert_eq!(page.records.len(), 1);
}

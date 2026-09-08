use assert_cmd::Command;
use serde_json::{json, Value};

fn request() -> Value {
    json!({
        "stable_prefix":{"instructions":["stable",{"v":1}]},
        "task":{"id":"continue","text":"Finish only this task"},
        "budget_tokens":1000,
        "items":[
            {"id":"must","content":{"evidence":"preserve"},"required":true},
            {"id":"later","content":"optional source","required":false,"priority":1},
            {"id":"first","content":"higher priority source","required":false,"priority":9}
        ]
    })
}

fn write_request(value: &Value) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("request.json");
    std::fs::write(&path, value.to_string()).unwrap();
    (dir, path)
}

#[test]
fn prepare_packet_only_preserves_json_values_without_audit_metadata() {
    let (_dir, path) = write_request(&request());
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file"])
        .arg(path)
        .arg("--packet-only")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let packet: Value = serde_json::from_slice(&output).unwrap();
    let raw = String::from_utf8(output).unwrap();
    assert!(raw.starts_with("{\"stable_prefix\":"));
    assert_eq!(packet["stable_prefix"], request()["stable_prefix"]);
    assert_eq!(packet["task"], request()["task"]);
    assert!(packet.get("source_sha256").is_none());
    assert_eq!(packet["items"][0]["id"], "must");
}

#[test]
fn packet_only_keeps_the_stable_prefix_at_the_same_leading_position() {
    let original = request();
    let (_dir, original_path) = write_request(&original);
    let mut changed = original;
    changed["items"].as_array_mut().unwrap().pop();
    let (_dir, changed_path) = write_request(&changed);
    let output = |path: &std::path::Path| {
        Command::cargo_bin("innen")
            .unwrap()
            .args(["middleware", "prepare", "--file"])
            .arg(path)
            .arg("--packet-only")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };
    let original = String::from_utf8(output(&original_path)).unwrap();
    let changed = String::from_utf8(output(&changed_path)).unwrap();
    let stable = "{\"stable_prefix\":{\"instructions\":[\"stable\",{\"v\":1}]},\"task\":";
    assert!(original.starts_with(stable));
    assert!(changed.starts_with(stable));
}

#[test]
fn prepare_receipt_and_expand_bind_the_same_source_hash() {
    let (_dir, path) = write_request(&request());
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file"])
        .arg(&path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prepared: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(prepared["resolution"], "prepared");
    assert_eq!(prepared["reference_tokenizer"], "o200k_base");
    assert_eq!(prepared["selected_ids"], json!(["must", "later", "first"]));
    let hash = prepared["source_sha256"].as_str().unwrap();
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "expand", "--file"])
        .arg(path)
        .args(["--expect-source-sha256", hash, "--ids", "first,must"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let expanded: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(expanded["source_sha256"], prepared["source_sha256"]);
    assert_eq!(expanded["items"][0]["id"], "must");
    assert_eq!(expanded["items"][1]["id"], "first");
    assert_eq!(expanded["items"][1]["content"], "higher priority source");
}

#[test]
fn required_over_budget_and_stale_source_are_unknown_not_truncated() {
    let mut too_small = request();
    too_small["budget_tokens"] = json!(1);
    let (_dir, path) = write_request(&too_small);
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file"])
        .arg(&path)
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let unknown: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(unknown["resolution"], "unknown");
    assert_eq!(unknown["reason"], "required_context_exceeds_budget");

    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "expand", "--file"])
        .arg(path)
        .args(["--expect-source-sha256", "0", "--ids", "must"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let unknown: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(unknown["reason"], "source_hash_mismatch");
}

#[test]
fn packet_only_failure_has_no_stdout_and_unknown_request_fields_are_rejected() {
    let mut too_small = request();
    too_small["budget_tokens"] = json!(1);
    let (_dir, path) = write_request(&too_small);
    Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file"])
        .arg(&path)
        .arg("--packet-only")
        .assert()
        .code(2)
        .stdout("");

    let mut invalid = request();
    invalid["unrecorded_constraint"] = json!(true);
    let (_dir, path) = write_request(&invalid);
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file"])
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
}

#[test]
fn stdin_admission_stops_before_an_oversized_request_is_parsed() {
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "prepare", "--file", "-"])
        .write_stdin("x".repeat(16 * 1024 * 1024 + 1))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds 16 MiB admission bound"));
}

use assert_cmd::Command;
use serde_json::{json, Value};

#[test]
fn native_grammar_packet_roundtrip_and_hash_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("page.json");
    let packet = temp.path().join("packet.json");
    let shared =
        "Source evidence remains unknown; do not treat process success as task completion.\n"
            .repeat(30);
    let records:Vec<_>=(0..6).map(|i|json!({"line":i+1,"event":{"role":"tool","text":format!("version {i}\n{shared}NOT APPROVED\n{shared}")}})).collect();
    let original = json!({"session_id":"fixture","records":records,"warnings":["partial source"],"next_offset":12});
    std::fs::write(&source, original.to_string()).unwrap();
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["conversation", "--encode-json"])
        .arg(&source)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let packed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(packed["encoding"], "innen.conversation-grammar.v1");
    std::fs::write(&packet, &output).unwrap();
    let decoded = Command::cargo_bin("innen")
        .unwrap()
        .args(["conversation", "--decode-packet"])
        .arg(&packet)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serde_json::from_slice::<Value>(&decoded).unwrap(), original);
    let checked = Command::cargo_bin("innen")
        .unwrap()
        .args(["conversation", "--validate-packet"])
        .arg(&packet)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serde_json::from_slice::<Value>(&checked).unwrap(), packed);
    let mut corrupt = packed;
    corrupt["source_sha256"] = json!("bad");
    std::fs::write(&packet, corrupt.to_string()).unwrap();
    Command::cargo_bin("innen")
        .unwrap()
        .args(["conversation", "--validate-packet"])
        .arg(&packet)
        .assert()
        .failure();
}

#[test]
fn native_source_route_preserves_page_and_default_interface() {
    let temp = tempfile::tempdir().unwrap();
    let uuid = "e3a92b35-4931-427b-9adf-1baa29318ca6";
    let dir = temp.path().join(uuid).join(".system_generated/logs");
    std::fs::create_dir_all(&dir).unwrap();
    let rows=(0..5).map(|i|json!({"step_index":i,"type":"USER_INPUT","content":format!("version {i}: {}", "共同證據不得省略否定與順序。\n".repeat(100))}).to_string()).collect::<Vec<_>>().join("\n");
    std::fs::write(dir.join("transcript.jsonl"), rows).unwrap();
    let args = ["conversation", uuid, "--source", "agy", "--source-root"];
    let plain = Command::cargo_bin("innen")
        .unwrap()
        .args(args)
        .arg(temp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let packed = Command::cargo_bin("innen")
        .unwrap()
        .args(args)
        .arg(temp.path())
        .args(["--compact", "--deltas", "--codec", "conversation"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let packed: Value = serde_json::from_slice(&packed).unwrap();
    assert_eq!(
        innen_core::conversation::grammar::decode(&packed).unwrap(),
        serde_json::from_slice::<Value>(&plain).unwrap()
    );
    Command::cargo_bin("innen")
        .unwrap()
        .args(args)
        .arg(temp.path())
        .args(["--codec", "conversation", "--attachment-refs"])
        .assert()
        .failure();
}

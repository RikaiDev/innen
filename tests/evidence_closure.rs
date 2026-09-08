use assert_cmd::Command;
use innen_core::graph::materialize;
use innen_core::ids::sha256_hex;
use serde_json::{json, Value};
use std::path::Path;

fn fixture(root: &Path) -> Value {
    let at = "2026-01-03T00:00:00Z";
    let mut events = Vec::new();
    for (id, body) in [
        ("claim", "decision statement"),
        ("rule", "governing rule"),
        ("source", "SECRET_SOURCE_PAYLOAD"),
    ] {
        events.push(json!({"id":format!("event-{id}"),"op":"node.upsert","observed_utc":at,
            "payload":{"id":id,"type":"Decision","label":id,"body":body,"provenance":"fixture:source"}}));
    }
    for (from, to) in [("claim", "rule"), ("rule", "source")] {
        events.push(
            json!({"id":format!("edge-{from}"),"op":"edge.assert","observed_utc":at,
            "payload":{"from":from,"to":to,"type":"DEPENDS_ON","provenance":"fixture:path"}}),
        );
    }
    let path = root.join(".innen");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("journal.jsonl"),
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let snapshot = materialize(&events, Some(at), false);
    let sources: serde_json::Map<_, _> = snapshot
        .nodes
        .iter()
        .map(|(id, n)| {
            (
                id.clone(),
                json!({"node_sha256":sha256_hex(n.to_string().as_bytes())}),
            )
        })
        .collect();
    json!({"version":1,"question":"What supports this decision?","scope_receipt":"fixture:authorized-scope",
        "scope_nodes":["claim","rule","source"],"as_of":at,"min_observed_utc":"2026-01-01T00:00:00Z",
        "sources":sources,"facts":[{"id":"decision","alternatives":[["claim"]]}],
        "budget_tokens":10000,"max_hops":4,"search_limit":1000})
}

fn run(root: &Path, c: &Value) -> std::process::Output {
    let path = root.join("contract.json");
    std::fs::write(&path, c.to_string()).unwrap();
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args([
            "query",
            "--q",
            "What supports this decision?",
            "--evidence-contract",
        ])
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn query_contract_closes_real_journal_and_never_mutates_it() {
    let dir = tempfile::tempdir().unwrap();
    let c = fixture(dir.path());
    let path = dir.path().join(".innen/journal.jsonl");
    let before = std::fs::read(&path).unwrap();
    let r = run(dir.path(), &c);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(out["resolution"], "covered");
    assert_eq!(out["packet"]["evidence"].as_array().unwrap().len(), 3);
    assert_eq!(
        out["packet"]["dependency_paths"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        out["packet_tokens"].as_u64().unwrap() as usize,
        innen_core::conversation::grammar::tokens(&out["packet"]).unwrap()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn missing_pin_expands_then_scope_denial_stays_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = fixture(dir.path());
    c["sources"].as_object_mut().unwrap().remove("source");
    let r = run(dir.path(), &c);
    assert_eq!(r.status.code(), Some(2));
    let out: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(out["source_requests"][0]["node"], "source");
    assert!(!String::from_utf8_lossy(&r.stdout).contains("SECRET_SOURCE_PAYLOAD"));
    c["scope_nodes"] = json!(["claim", "rule"]);
    let r = run(dir.path(), &c);
    assert_eq!(r.status.code(), Some(2));
    let out: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert!(out["source_requests"].as_array().unwrap().is_empty());
    assert!(out["rejected_alternatives"][0]["source"].is_null());
}

#[test]
fn corrupted_journal_and_mismatched_question_fail_without_repair() {
    let dir = tempfile::tempdir().unwrap();
    let c = fixture(dir.path());
    let path = dir.path().join(".innen/journal.jsonl");
    let mut raw = std::fs::read(&path).unwrap();
    raw.extend_from_slice(b"\n{bad");
    std::fs::write(&path, &raw).unwrap();
    let r = run(dir.path(), &c);
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(std::fs::read(&path).unwrap(), raw);
    let mut c = fixture(dir.path());
    c["question"] = json!("Different task");
    assert_eq!(run(dir.path(), &c).status.code(), Some(1));
}

#[test]
fn evidence_view_exposes_hash_consumed_by_contract() {
    let dir = tempfile::tempdir().unwrap();
    let c = fixture(dir.path());
    let r = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(dir.path())
        .args(["query", "--q", "claim", "--view", "evidence"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out: Value = serde_json::from_slice(&r).unwrap();
    let candidate = out["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "claim")
        .unwrap();
    assert_eq!(
        candidate["node_sha256"],
        c["sources"]["claim"]["node_sha256"]
    );
}

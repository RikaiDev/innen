use assert_cmd::Command;
use innen_core::ids::sha256_hex;
use serde_json::{json, Value};

fn accepted_review(out: &Value) -> Value {
    let cite = &out["requirements"][0]["citations"][0];
    let span = json!({
        "source": cite["source"],
        "pointer": cite["pointer"],
        "start_byte": cite["start_byte"],
        "end_byte": cite["end_byte"],
        "node_sha256": cite["node_sha256"],
    });
    json!({
        "context_sha256": out["context_sha256"],
        "proposal_sha256": out["proposal_sha256"],
        "accepted": true,
        "reviewer": "test-controller",
        "reason": "Matches fixture source and read-only scope",
        "premises": [{
            "id": "build-boundary",
            "claim": "The build failed and deployment is not authorized",
            "supports": [span],
            "refutes": [],
        }],
    })
}

#[test]
fn native_proposal_prepare_validate_review_and_closure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join(".innen")).unwrap();
    let at = "2026-01-03T00:00:00Z";
    let node = json!({"id":"source","type":"Decision","label":"source","body":"Build failed; deployment is not authorized.","provenance":"fixture:report","observed_utc":at});
    std::fs::write(
        root.join(".innen/journal.jsonl"),
        json!({"id":"e1","observed_utc":at,"op":"node.upsert","payload":node}).to_string(),
    )
    .unwrap();
    let scope = json!({"version":1,"question":"Continue the investigation","scope_receipt":"fixture:scope","scope_nodes":["source"],"as_of":at,"min_observed_utc":at,"sources":{"source":{"node_sha256":sha256_hex(node.to_string().as_bytes())}},"facts":[],"budget_tokens":5000,"max_hops":2,"search_limit":100});
    let path = root.join("scope.json");
    std::fs::write(&path, scope.to_string()).unwrap();
    let invoke = |extra: &[&str]| {
        let out = Command::cargo_bin("innen")
            .unwrap()
            .arg("--root")
            .arg(root)
            .args([
                "query",
                "--q",
                "Continue the investigation",
                "--evidence-contract",
            ])
            .arg(&path)
            .args(extra)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        )
    };
    let (code, context) = invoke(&["--prepare-proposal"]);
    assert_eq!(code, 0);
    let p = json!({"context_sha256":context["context_sha256"],"question":scope["question"],"goal":"Investigate the build failure","disposition":"proceed_with_investigation","requirements":[{"id":"failure","text":"Preserve failure and authorization boundary","citations":[{"source":"source","pointer":"/body","quote":"Build failed; deployment is not authorized."}],"alternatives":[["source"]]}],"unknowns":[]});
    let pp = root.join("proposal.json");
    std::fs::write(&pp, p.to_string()).unwrap();
    let (code, out) = invoke(&["--task-proposal", pp.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert_eq!(out["resolution"], "source_validated_review_required");
    let rp = root.join("review.json");
    std::fs::write(&rp, accepted_review(&out).to_string()).unwrap();
    let (code, out) = invoke(&[
        "--task-proposal",
        pp.to_str().unwrap(),
        "--proposal-review",
        rp.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert_eq!(out["resolution"], "covered");

    let (code, context) = invoke(&["--prepare-proposal", "--evidence-ids"]);
    assert_eq!(code, 0);
    assert_eq!(context["evidence_catalog"][0]["evidence_id"], "E1");
    let dp = root.join("draft.json");
    let mut draft = json!({"context_sha256":context["context_sha256"],"draft":{
        "goal":"Investigate only","disposition":"proceed_with_investigation",
        "requirements":[{"id":"boundary","text":"Build failed; deployment is not authorized.","evidence_ids":["E1"]}],"unknowns":[]}});
    std::fs::write(&dp, draft.to_string()).unwrap();
    let (code, out) = invoke(&["--task-draft", dp.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert_eq!(out["resolution"], "source_validated_review_required");
    std::fs::write(&rp, accepted_review(&out).to_string()).unwrap();
    let (code, out) = invoke(&[
        "--task-draft",
        dp.to_str().unwrap(),
        "--proposal-review",
        rp.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert_eq!(out["resolution"], "covered");
    draft["context_sha256"] = json!("stale-context");
    std::fs::write(&dp, draft.to_string()).unwrap();
    Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args([
            "query",
            "--q",
            "Continue the investigation",
            "--evidence-contract",
        ])
        .arg(&path)
        .arg("--task-draft")
        .arg(&dp)
        .assert()
        .failure();
}

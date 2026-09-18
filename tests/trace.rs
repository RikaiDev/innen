use assert_cmd::Command;
use innen_core::journal::Journal;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::Duration;

fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("innen")
        .unwrap()
        .timeout(Duration::from_secs(30))
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}
fn output(root: &Path, args: &[&str]) -> Value {
    let result = cli(root, args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn page(root: &Path, source: &Path) {
    fs::create_dir_all(root.join("02-wiki/ai")).unwrap();
    fs::write(root.join("02-wiki/ai/release.md"), format!(
        "---\ntitle: Release deployment history\ntags: [release]\ncreated: 2026-01-01\nupdated: 2026-01-01\nsources:\n  - '{}'\nrelated: []\n---\nRelease develop main deployment decisions.\n", source.display())).unwrap();
}
fn expand(root: &Path, hit: &Value) -> std::process::Output {
    let args: Vec<&str> = hit["expand_argv"]
        .as_array()
        .unwrap()
        .iter()
        .skip(3)
        .map(|v| v.as_str().unwrap())
        .collect();
    cli(root, &args)
}

#[test]
fn wiki_clue_reaches_source_then_warm_cache_and_exact_expansion() {
    let kb = tempfile::tempdir().unwrap();
    let source = kb.path().join("release-history.md");
    fs::write(&source, "## User\nRelease policy: develop integrates; main deploys only approved tags.\n## Assistant\nA different release proposal, not a user decision.\n").unwrap();
    page(kb.path(), &source);
    output(kb.path(), &["wiki", "sync"]);
    let args = ["trace", "--q", "release develop main", "--role", "user"];
    let cold = output(kb.path(), &args);
    let hit = &cold["hits"][0];
    assert_eq!(hit["role"], "user");
    assert!(hit["quote"].as_str().unwrap().contains("approved tags"));
    assert!(hit["path"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["edge"] == "DERIVED_FROM"));
    assert!(expand(kb.path(), hit).status.success());
    let warm = output(kb.path(), &args);
    assert_eq!(warm["hits"][0]["record_sha256"], hit["record_sha256"]);
    assert!(warm["work"]["source_cache_hits"].as_u64().unwrap() > 0);
    fs::write(
        &source,
        "## User\nCorrection: no deployment authorization exists.\n",
    )
    .unwrap();
    let stale = expand(kb.path(), hit);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("mismatch"));
}

#[test]
fn harvested_bare_provenance_reaches_the_inbox_source() {
    let kb = tempfile::tempdir().unwrap();
    let inbox = kb.path().join("00-inbox/harvest");
    fs::create_dir_all(&inbox).unwrap();
    let source = inbox.join("browser-agent-history.md");
    let text = "Browser Agent Windows bundle was delivered for the station.\n";
    fs::write(&source, text).unwrap();

    let journal = Journal::open(kb.path()).unwrap();
    journal
        .append(
            "node.upsert",
            &json!({
                "id":"source:history",
                "type":"Artifact",
                "label":"browser-agent-history.md",
                "body":text,
                "provenance":{"path":"browser-agent-history.md","bytes":text.len()}
            }),
        )
        .unwrap();
    drop(journal);

    let out = output(kb.path(), &["trace", "--q", "browser agent Windows"]);
    assert!(out["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit["quote"].as_str().unwrap().contains("delivered")));
    assert!(out["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["status"] == "searched"));
    assert!(out["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| { hit["source_path"].as_str() == Some(source.to_str().unwrap()) }));
}

#[test]
fn trace_accepts_explicit_utf8_sources_without_an_extension_or_with_log_extension() {
    for filename in ["release-history", "release-history.log"] {
        let kb = tempfile::tempdir().unwrap();
        let source = kb.path().join(filename);
        fs::write(
            &source,
            "## User\nRelease policy: develop integrates; main deploys only approved tags.\n",
        )
        .unwrap();
        page(kb.path(), &source);
        output(kb.path(), &["wiki", "sync"]);

        let out = output(kb.path(), &["trace", "--q", "release develop main"]);
        assert!(out["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| { hit["quote"].as_str().unwrap().contains("approved tags") }));
        assert!(!out["sources"].as_array().unwrap().iter().any(|source| {
            source["status"] == "unavailable"
                && source["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("unsupported source type"))
        }));
    }
}

#[test]
fn reverse_delivery_finds_native_user_and_preserves_physical_source_identity() {
    let kb = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let id = "00000000-0000-4000-8000-000000000011";
    let path = native.path().join(format!("{id}.jsonl"));
    let events = [
        json!({"type":"session_meta","payload":{"id":id}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Release must use develop before main; retain the rollback receipt."}]}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"Release must use develop before main; retain the rollback receipt."}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"release develop main alternative proposal"}]}}),
    ];
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let journal = Journal::open(kb.path()).unwrap();
    journal.append("node.upsert",&json!({"id":"artifact:release","type":"Artifact","label":"release develop main policy"})).unwrap();
    journal.append("node.upsert",&json!({"id":format!("conversation:codex:{id}"),"type":"Conversation","label":"origin conversation"})).unwrap();
    journal.append("edge.assert",&json!({"from":format!("conversation:codex:{id}"),"type":"DELIVERED","to":"artifact:release","provenance":"fixture delivery receipt"})).unwrap();
    drop(journal);
    let out = output(
        kb.path(),
        &[
            "trace",
            "--q",
            "release develop main",
            "--seed",
            "artifact:release",
            "--source-root",
            native.path().to_str().unwrap(),
            "--role",
            "user",
        ],
    );
    assert_eq!(out["hits"].as_array().unwrap().len(), 1);
    let hit = &out["hits"][0];
    assert_eq!(hit["line"], 2);
    assert_eq!(hit["path"][0]["direction"], "reverse");
    let expanded = expand(kb.path(), hit);
    assert!(
        expanded.status.success(),
        "{}",
        String::from_utf8_lossy(&expanded.stderr)
    );
    let expanded: Value = serde_json::from_slice(&expanded.stdout).unwrap();
    assert!(expanded["record"]["content"]
        .as_str()
        .unwrap()
        .contains("rollback receipt"));
}

#[test]
fn native_command_stdout_keeps_ordinal_pointer_and_scalar_hash() {
    let kb = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let id = "00000000-0000-4000-8000-000000000012";
    let path = native.path().join(format!("{id}.jsonl"));
    let output_text = "CORRECT_COMMAND_OUTPUT";
    let events = [
        json!({"type":"event_msg","ordinal":17,"payload":{"type":"item_completed","item":{"type":"command_execution","aggregated_output":output_text}}}),
    ];
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let journal = Journal::open(kb.path()).unwrap();
    journal
        .append(
            "node.upsert",
            &json!({"id":"artifact:cmd","type":"Artifact","label":"command evidence"}),
        )
        .unwrap();
    journal.append("node.upsert", &json!({"id":format!("conversation:codex:{id}"),"type":"Conversation","label":"native command"})).unwrap();
    journal.append("edge.assert", &json!({"from":format!("conversation:codex:{id}"),"type":"DELIVERED","to":"artifact:cmd","provenance":"fixture"})).unwrap();
    drop(journal);
    let out = output(
        kb.path(),
        &[
            "trace",
            "--q",
            "CORRECT_COMMAND_OUTPUT",
            "--seed",
            "artifact:cmd",
            "--source-root",
            native.path().to_str().unwrap(),
        ],
    );
    let hit = &out["hits"][0];
    assert_eq!(hit["ordinal"], 17);
    assert_eq!(hit["json_pointer"], "/payload/item/aggregated_output");
    assert_eq!(
        hit["scalar_sha256"],
        innen_core::ids::sha256_hex(output_text.as_bytes())
    );
    assert!(hit["quote"].as_str().unwrap().contains(output_text));
}

#[test]
fn missing_sources_and_budget_exhaustion_are_not_empty_success_claims() {
    let kb = tempfile::tempdir().unwrap();
    page(kb.path(), &kb.path().join("does-not-exist.md"));
    output(kb.path(), &["wiki", "sync"]);
    let missing = output(kb.path(), &["trace", "--q", "release"]);
    assert!(missing["hits"].as_array().unwrap().is_empty());
    assert_eq!(missing["coverage_incomplete"], true);
    assert!(missing["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["status"] == "unavailable"));
    fs::write(
        kb.path().join("does-not-exist.md"),
        "x".repeat(1000) + "release",
    )
    .unwrap();
    let limited = output(kb.path(), &["trace", "--q", "release", "--max-bytes", "16"]);
    assert_eq!(limited["coverage_incomplete"], true);
    let no_query = cli(kb.path(), &["trace", "--q", ""]);
    assert!(!no_query.status.success());
}

#[test]
fn changed_frontmatter_retracts_old_route_without_deleting_source() {
    let kb = tempfile::tempdir().unwrap();
    let old = kb.path().join("old.md");
    let new = kb.path().join("new.md");
    fs::write(&old, "release old proposal").unwrap();
    fs::write(&new, "release corrected decision").unwrap();
    page(kb.path(), &old);
    output(kb.path(), &["wiki", "sync"]);
    page(kb.path(), &new);
    output(kb.path(), &["wiki", "sync"]);
    let out = output(kb.path(), &["trace", "--q", "release corrected"]);
    assert!(out["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["quote"].as_str().unwrap().contains("corrected decision")));
    assert!(old.exists());
}

#[test]
fn invalid_delivery_provenance_does_not_create_a_source_route() {
    let kb = tempfile::tempdir().unwrap();
    let journal = Journal::open(kb.path()).unwrap();
    journal
        .append(
            "node.upsert",
            &json!({"id":"artifact:one","type":"Artifact","label":"release"}),
        )
        .unwrap();
    journal.append("node.upsert", &json!({"id":"conversation:codex:00000000-0000-4000-8000-000000000001","type":"Conversation","label":"origin"})).unwrap();
    journal.append("edge.assert", &json!({"from":"conversation:codex:00000000-0000-4000-8000-000000000001","type":"DELIVERED","to":"artifact:one"})).unwrap();
    drop(journal);
    let out = output(
        kb.path(),
        &["trace", "--q", "release", "--seed", "artifact:one"],
    );
    assert!(out["hits"].as_array().unwrap().is_empty());
    assert_eq!(out["work"]["sources_attempted"], 0);
}

#[test]
fn remote_reference_is_reported_as_unsearched_coverage() {
    let kb = tempfile::tempdir().unwrap();
    fs::create_dir_all(kb.path().join("02-wiki")).unwrap();
    fs::write(kb.path().join("02-wiki/release.md"),"---\ntitle: Release\ntags: [release]\nsources: [https://example.test/release]\nrelated: []\n---\nrelease policy\n").unwrap();
    output(kb.path(), &["wiki", "sync"]);
    let out = output(kb.path(), &["trace", "--q", "release"]);
    assert_eq!(out["coverage_incomplete"], true);
    assert!(out["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["status"] == "remote_reference_not_fetched"));
}

#[test]
fn failed_scan_charges_the_remaining_budget_and_does_not_claim_measured_zero() {
    let kb = tempfile::tempdir().unwrap();
    let source = kb.path().join("release-invalid.md");
    fs::write(&source, [b'r', b'e', b'l', 0xff]).unwrap();
    page(kb.path(), &source);
    output(kb.path(), &["wiki", "sync"]);
    let out = output(
        kb.path(),
        &["trace", "--q", "release", "--max-bytes", "8192"],
    );
    assert!(out["work"]["source_bytes_read"].is_null());
    assert_eq!(out["work"]["source_bytes_charged"], 8192);
    assert_eq!(out["work"]["failed_reads_with_unknown_consumption"], 1);
}

#[test]
fn wiki_receipt_session_descriptor_reaches_the_original_conversation() {
    let kb = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let id = "00000000-0000-4000-8000-000000000077";
    let events = [
        json!({"type":"session_meta","payload":{"id":id}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Release develop main needs the original approval record."}]}}),
    ];
    fs::write(
        native.path().join(format!("{id}.jsonl")),
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let receipt = kb.path().join("release-receipt.json");
    fs::write(
        &receipt,
        json!({"source":"codex","session_id":id,"summary":"release integration receipt"})
            .to_string(),
    )
    .unwrap();
    page(kb.path(), &receipt);
    output(kb.path(), &["wiki", "sync"]);
    let out = output(
        kb.path(),
        &[
            "trace",
            "--q",
            "release develop main",
            "--role",
            "user",
            "--source-root",
            native.path().to_str().unwrap(),
        ],
    );
    let hit = &out["hits"][0];
    assert_eq!(hit["role"], "user");
    assert_eq!(hit["line"], 2);
    assert!(hit["path"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["edge"] == "METADATA_SESSION_REF"
            && p["provenance"].as_str().unwrap().ends_with("#/session_id")));
    assert!(expand(kb.path(), hit).status.success());
}

use super::archive::{antigravity_targets, hash_targets};
use super::delete_adapter::{delete_candidate, run_delete, run_sqlite_mutation};
use super::policy::is_trustworthy_modified;
use super::*;

const ID: &str = "e3a92b35-4931-427b-9adf-1baa29318ca6";

fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let kb = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let project = store.path().join("-project");
    fs::create_dir_all(&project).unwrap();
    let source = project.join(format!("{ID}.jsonl"));
    fs::write(
        &source,
        concat!(
            "{\"type\":\"user\",\"timestamp\":\"2020-01-01T00:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"question\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2020-01-01T00:01:00Z\",\"message\":{\"role\":\"assistant\",\"content\":\"answer\"}}\n"
        ),
    )
    .unwrap();
    let journal = Journal::open(kb.path()).unwrap();
    journal
        .append(
            "node.upsert",
            &json!({"id":format!("conversation:claude:{ID}"),"type":"Conversation","label":"session"}),
        )
        .unwrap();
    journal
        .append(
            "node.upsert",
            &json!({"id":"artifact:knowledge","type":"Artifact","label":"knowledge"}),
        )
        .unwrap();
    journal
        .append(
            "edge.assert",
            &json!({"from":"artifact:knowledge","type":"DERIVED_FROM","to":format!("conversation:claude:{ID}")}),
        )
        .unwrap();
    (kb, store, source)
}

#[test]
fn age_and_harvest_are_not_enough_but_exact_attestation_is() {
    let (kb, store, _source) = setup();
    let before = inventory(kb.path(), Some(Source::Claude), Some(store.path()), 45).unwrap();
    assert_eq!(before.len(), 1);
    assert!(!before[0].eligible);
    assert!(before[0].blockers[0].contains("attestation"));
    assert!(before[0]
        .blocker_codes
        .contains(&BLOCKER_ATTESTATION_MISSING.to_string()));

    attest(
        kb.path(),
        "claude",
        ID,
        Some(store.path()),
        &["artifact:knowledge".into()],
        true,
    )
    .unwrap();
    let after = inventory(kb.path(), Some(Source::Claude), Some(store.path()), 45).unwrap();
    assert!(after[0].eligible, "{:?}", after[0].blockers);
}

#[test]
fn source_change_invalidates_proof_and_execute_never_deletes_it() {
    let (kb, store, source) = setup();
    attest(
        kb.path(),
        "claude",
        ID,
        Some(store.path()),
        &["artifact:knowledge".into()],
        true,
    )
    .unwrap();
    fs::write(&source, b"changed\n").unwrap();
    let error = purge(kb.path(), "claude", ID, Some(store.path()), 45, true).unwrap_err();
    assert!(
        error.to_string().contains("retention window") || error.to_string().contains("attestation"),
        "{error}"
    );
    assert!(source.exists());
    let graph = load_graph(kb.path()).unwrap();
    assert!(graph
        .nodes
        .values()
        .any(|node| { node.get("type").and_then(Value::as_str) == Some("SessionPurgeAttempt") }));
    assert!(graph
        .nodes
        .values()
        .any(|node| { node.get("type").and_then(Value::as_str) == Some("SessionPurgeFailure") }));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn open_native_session_is_blocked() {
    let (kb, store, source) = setup();
    attest(
        kb.path(),
        "claude",
        ID,
        Some(store.path()),
        &["artifact:knowledge".into()],
        true,
    )
    .unwrap();
    let _open = File::open(source).unwrap();
    let assessment = inventory(kb.path(), Some(Source::Claude), Some(store.path()), 45)
        .unwrap()
        .pop()
        .unwrap();
    assert!(!assessment.eligible);
    assert!(assessment
        .blocker_codes
        .contains(&BLOCKER_SESSION_ACTIVE.to_string()));
}

#[test]
fn antigravity_bundle_includes_conversation_database_and_brain() {
    let store = tempfile::tempdir().unwrap();
    let brain = store.path().join("brain").join(ID);
    let conversations = store.path().join("conversations");
    fs::create_dir_all(&brain).unwrap();
    fs::create_dir_all(&conversations).unwrap();
    fs::write(brain.join("note.md"), b"knowledge").unwrap();
    let db = conversations.join(format!("{ID}.db"));
    fs::write(&db, b"trajectory").unwrap();
    let targets = antigravity_targets(&db, ID);
    assert_eq!(targets, vec![brain, db]);
    let hashed = hash_targets(&targets).unwrap();
    assert_eq!(hashed.bytes, 19);
}

#[test]
fn delete_adapter_captures_success_output() {
    run_delete("/bin/sh", &["-c", "printf adapter-noise"]).unwrap();
}

#[test]
fn inventory_summary_is_bounded_and_aggregated() {
    let make = |id: &str, eligible: bool, bytes: u64| Assessment {
        session_id: id.into(),
        source: "codex".into(),
        modified: None,
        cutoff_utc: String::new(),
        source_bytes: bytes,
        source_sha256: String::new(),
        eligible,
        blockers: if eligible { vec![] } else { vec!["hot".into()] },
        blocker_codes: if eligible {
            vec![]
        } else {
            vec![BLOCKER_RETENTION_WINDOW.into()]
        },
        targets: vec![PathBuf::from(format!("/private/{id}"))],
    };
    let summary = summarize_inventory(&[make("hot", false, 0), make("cold", true, 42)]);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.eligible, 1);
    assert_eq!(summary.eligible_bytes, 42);
    assert_eq!(summary.by_source["codex"].total, 2);
    assert_eq!(summary.blocker_codes[BLOCKER_RETENTION_WINDOW], 1);
    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("private/"));
    assert!(!json.contains("session_id"));
}

#[test]
fn placeholder_modified_year_is_not_deletion_evidence() {
    assert!(!is_trustworthy_modified(Some("0001-01-01 00:00:00+00:00")));
    assert!(is_trustworthy_modified(Some("2026-01-01T00:00:00Z")));
}

#[test]
fn antigravity_delete_removes_bundle_and_summary_row() {
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("antigravity-cli");
    let brain = store.join("brain").join(ID);
    let conversations = store.join("conversations");
    fs::create_dir_all(&brain).unwrap();
    fs::create_dir_all(&conversations).unwrap();
    fs::write(brain.join("note.md"), b"knowledge").unwrap();
    let db = conversations.join(format!("{ID}.db"));
    fs::write(&db, b"trajectory").unwrap();
    let summary = store.join("conversation_summaries.db");
    run_sqlite_mutation(
        &summary,
        &format!("CREATE TABLE conversation_summaries (conversation_id TEXT PRIMARY KEY, not_fully_idle INTEGER, killed INTEGER); INSERT INTO conversation_summaries VALUES ('{ID}',0,0);"),
    )
    .unwrap();
    let candidate = Candidate {
        id: ID.into(),
        source: Source::Antigravity,
        modified: Some("2020-01-01T00:00:00Z".into()),
        path: db.clone(),
        project: None,
        parent_id: None,
    };
    let bundle = bundle(&candidate).unwrap();
    let assessment = Assessment {
        session_id: ID.into(),
        source: "antigravity".into(),
        modified: candidate.modified.clone(),
        cutoff_utc: String::new(),
        source_bytes: bundle.bytes,
        source_sha256: bundle.sha256,
        eligible: true,
        blockers: vec![],
        blocker_codes: vec![],
        targets: bundle.targets,
    };
    delete_candidate(&candidate, &assessment).unwrap();
    assert!(!brain.exists());
    assert!(!db.exists());
    let rows = sources::sqlite(
        &summary,
        &format!("SELECT json_object('id',conversation_id) FROM conversation_summaries WHERE conversation_id='{ID}'"),
    )
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn sweep_dry_run_is_bounded_and_does_not_delete() {
    let (kb, store, source) = setup();
    attest(
        kb.path(),
        "claude",
        ID,
        Some(store.path()),
        &["artifact:knowledge".into()],
        true,
    )
    .unwrap();
    let receipt = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        false,
    )
    .unwrap();
    assert_eq!(receipt.total, 1);
    assert_eq!(receipt.eligible, 1);
    assert_eq!(receipt.removed, 0);
    assert_eq!(receipt.failed, 0);
    assert_eq!(receipt.items.len(), 1);
    assert!(source.exists());
}

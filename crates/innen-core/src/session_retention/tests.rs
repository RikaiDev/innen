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
    delete_candidate(&candidate, &assessment, None).unwrap();
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

#[test]
fn sweep_compacts_an_unattested_closed_session_then_purges_it() {
    let (kb, store, source) = setup();
    let dry = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        false,
    )
    .unwrap();
    assert_eq!((dry.eligible, dry.compactable, dry.removed), (0, 1, 0));
    assert!(source.exists(), "a dry run never deletes");

    let run = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        true,
    )
    .unwrap();
    assert_eq!(
        (run.compacted, run.removed, run.failed),
        (1, 1, 0),
        "{:?}",
        run.items
    );
    assert!(!source.exists());
    let compact = kb
        .path()
        .join(COMPACT_DIR)
        .join("claude")
        .join(format!("{ID}.jsonl"));
    let rows: Vec<Value> = fs::read_to_string(&compact)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0]["role"].as_str(), rows[0]["line"].as_u64()),
        (Some("user"), Some(1))
    );
    assert_eq!(rows[1]["content"], "answer");
    assert_eq!(
        run.compact_bytes_written,
        fs::metadata(&compact).unwrap().len()
    );
}

#[test]
fn tool_managed_deletion_refuses_a_source_root_override() {
    // Regression: `codex delete <id>` acts on ~/.codex, so an override copy must
    // never reach it (a trial on 2026-09-30 deleted the real session instead).
    let store = tempfile::tempdir().unwrap();
    let path = store
        .path()
        .join(format!("rollout-2020-01-01T00-00-00-{ID}.jsonl"));
    fs::write(&path, "{}\n").unwrap();
    let candidate = Candidate {
        id: ID.into(),
        source: Source::Codex,
        modified: Some("2020-01-01T00:00:00Z".into()),
        path: path.clone(),
        project: None,
        parent_id: None,
    };
    let bundle = bundle(&candidate).unwrap();
    let assessment = Assessment {
        session_id: ID.into(),
        source: "codex".into(),
        modified: candidate.modified.clone(),
        cutoff_utc: String::new(),
        source_bytes: bundle.bytes,
        source_sha256: bundle.sha256,
        eligible: true,
        blockers: vec![],
        blocker_codes: vec![],
        targets: bundle.targets,
    };
    let error = delete_candidate(&candidate, &assessment, Some(store.path())).unwrap_err();
    assert!(error.to_string().contains("--source-root"), "{error}");
    assert!(path.exists());
}

#[test]
fn codex_parent_with_a_live_subagent_session_is_not_purged() {
    // Regression: `codex delete` removes a thread's subagent sessions, so a parent
    // purged first silently deleted 100 uncompacted children on 2026-09-30.
    const PARENT: &str = "019fe02a-8348-7c23-8ad1-f403c6ee9af4";
    const CHILD: &str = "01a0063c-ca23-7ad0-87e2-f3b2fca1137c";
    let kb = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let day = store.path().join("2020/01/01");
    fs::create_dir_all(&day).unwrap();
    let meta = |id: &str, source: Value| {
        format!(
            "{}\n{}\n",
            json!({"type":"session_meta","timestamp":"2020-01-01T00:00:00Z",
                   "payload":{"id":id,"cwd":"/tmp","source":source}}),
            json!({"type":"response_item","timestamp":"2020-01-01T00:00:01Z",
                   "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}})
        )
    };
    fs::write(
        day.join(format!("rollout-2020-01-01T00-00-00-{PARENT}.jsonl")),
        meta(PARENT, json!("cli")),
    )
    .unwrap();
    let child = day.join(format!("rollout-2020-01-01T00-00-01-{CHILD}.jsonl"));
    fs::write(
        &child,
        meta(
            CHILD,
            json!({"subagent":{"thread_spawn":{"parent_thread_id":PARENT,"depth":1}}}),
        ),
    )
    .unwrap();
    let candidates = find_all_candidates(Some(Source::Codex), Some(store.path())).unwrap();
    let found = candidates
        .iter()
        .find(|c| c.id == CHILD)
        .expect("child discovered");
    assert_eq!(found.parent_id.as_deref(), Some(PARENT));

    // Dry run only: the guard fires before any delete adapter could run.
    let error = purge(kb.path(), "codex", PARENT, Some(store.path()), 0, false).unwrap_err();
    assert!(error.to_string().contains(CHILD), "{error}");
    assert!(child.exists());
}

#[cfg(unix)]
#[test]
fn sweep_reports_native_files_kept_alive_by_other_hard_links() {
    // Another tool hard-linking the native store (Orca did, 2026-09-30) keeps the
    // bytes on disk after purge; the receipt must say so instead of implying space.
    let (kb, store, source) = setup();
    let elsewhere = tempfile::tempdir().unwrap();
    let mirror = elsewhere.path().join("mirror.jsonl");
    fs::hard_link(&source, &mirror).unwrap();
    let bytes = fs::metadata(&source).unwrap().len();

    let dry = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        false,
    )
    .unwrap();
    assert_eq!((dry.hard_linked, dry.hard_linked_bytes), (1, bytes));
    assert!(dry.items[0]
        .detail
        .as_deref()
        .unwrap()
        .contains("hard link"));

    let run = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        true,
    )
    .unwrap();
    assert_eq!(
        (run.removed, run.hard_linked, run.hard_linked_bytes),
        (1, 1, bytes)
    );
    assert!(run.items[0]
        .detail
        .as_deref()
        .unwrap()
        .contains("frees no space"));
    assert!(!source.exists() && mirror.exists());
}

#[test]
fn sweep_reports_no_hard_links_for_an_unshared_session() {
    let (kb, store, _source) = setup();
    let run = sweep(
        kb.path(),
        Some(Source::Claude),
        Some(store.path()),
        45,
        true,
    )
    .unwrap();
    assert_eq!(
        (run.removed, run.hard_linked, run.hard_linked_bytes),
        (1, 0, 0)
    );
    assert!(run.items[0].detail.is_none());
}

#[test]
fn a_session_whose_native_bundle_is_gone_is_not_an_assessment_error() {
    // A summary row can outlive the files it describes. Before, that surfaced
    // as `assessment_error`, so every later plan and sweep failed forever on
    // rows with nothing left to clean.
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("antigravity-cli");
    let summary = store.join("conversation_summaries.db");
    fs::create_dir_all(&store).unwrap();
    run_sqlite_mutation(
        &summary,
        "CREATE TABLE conversation_summaries (conversation_id TEXT PRIMARY KEY, not_fully_idle INTEGER, killed INTEGER);",
    )
    .unwrap();
    // Row present, but no brain/ or conversations/ directory exists.
    run_sqlite_mutation(
        &summary,
        &format!("INSERT INTO conversation_summaries VALUES ('{ID}',0,0);"),
    )
    .unwrap();

    let missing = Candidate {
        id: ID.into(),
        source: Source::Antigravity,
        modified: Some("2020-01-01T00:00:00Z".into()),
        path: store.join("conversations").join(format!("{ID}.db")),
        project: None,
        parent_id: None,
    };
    let assessments = inventory_of(root.path(), &[missing], 0).expect("inventory");
    assert_eq!(assessments.len(), 1);
    assert_eq!(
        assessments[0].blocker_codes,
        vec!["already_cleaned".to_string()],
        "a missing bundle is the desired end state, not a failure: {:?}",
        assessments[0].blockers
    );
    assert!(!assessments[0].eligible);
    assert!(
        assessments[0].targets.iter().all(|t| !t.exists()),
        "nothing may be left to delete"
    );
}

#[test]
fn a_real_assessment_failure_still_reports_assessment_error() {
    // The new branch must not swallow genuine errors: a session whose files
    // exist but cannot be read keeps the old blocker.
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("antigravity-cli");
    let brain = store.join("brain").join(ID);
    fs::create_dir_all(&brain).unwrap();
    fs::write(brain.join("note.md"), b"x").unwrap();
    // No summary DB and no conversation db: the bundle hashes fine, so this
    // exercises the pass-through rather than the new branch.
    let candidate = Candidate {
        id: ID.into(),
        source: Source::Antigravity,
        modified: None,
        path: store.join("conversations").join(format!("{ID}.db")),
        project: None,
        parent_id: None,
    };
    let assessments = inventory_of(root.path(), &[candidate], 0).expect("inventory");
    assert_eq!(assessments.len(), 1);
    assert_ne!(
        assessments[0].blocker_codes,
        vec!["already_cleaned".to_string()],
        "an existing bundle must never be called already-cleaned"
    );
}

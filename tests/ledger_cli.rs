use assert_cmd::Command;
use innen_core::graph::materialize;
use innen_core::journal::Journal;
use serde_json::json;

#[test]
fn ledger_add_mirrors_receipt_into_queryable_kb() {
    let kb = tempfile::tempdir().expect("kb");
    let journal = Journal::open(kb.path()).expect("journal");
    for (id, label) in [("project:one", "One"), ("project:archive", "Archive")] {
        journal
            .append(
                "node.upsert",
                &json!({"id":id,"type":"Project","label":label,"provenance":"fixture"}),
            )
            .expect("project node");
    }
    drop(journal);
    let workspace = tempfile::tempdir().expect("workspace");
    let archive = workspace.path().join("Ledger Project");
    let source = workspace.path().join("NHRI-report.pdf");
    std::fs::write(&source, b"verified source bytes").expect("source");
    let kb_root = kb.path().to_string_lossy().into_owned();
    let archive_root = archive.to_string_lossy().into_owned();
    let source_path = source.to_string_lossy().into_owned();

    Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "artifact",
            "ledger",
            "init",
            "--archive-root",
            &archive_root,
            "--project-id",
            "project:one",
            "--title",
            "Ledger Project",
        ])
        .assert()
        .success();
    Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "artifact",
            "ledger",
            "add",
            "--archive-root",
            &archive_root,
            "--file",
            &source_path,
            "--source-kind",
            "document",
            "--role",
            "reference",
            "--stage",
            "verification",
            "--owner-project",
            "project:one",
            "--archive-project-id",
            "project:archive",
            "--relation",
            "belongs_to",
            "--document-id",
            "document:nhri-report",
            "--provenance",
            "fixture source receipt",
        ])
        .assert()
        .success();

    let out = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "--format",
            "json",
            "query",
            "--q",
            "NHRI-report.pdf",
        ])
        .output()
        .expect("query process");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("utf8");
    assert!(text.contains("NHRI-report.pdf"));
    assert!(text.contains("project:one"));

    let out = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "--format",
            "json",
            "query",
            "--q",
            "document:nhri-report",
        ])
        .output()
        .expect("document query process");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("utf8");
    assert!(text.contains("document:nhri-report"));
}

#[test]
fn reference_receipt_and_reclassification_update_queryable_owner() {
    let kb = tempfile::tempdir().expect("kb");
    let journal = Journal::open(kb.path()).expect("journal");
    for (id, label) in [
        ("project:archive", "Archive"),
        ("project:old", "Old"),
        ("project:new", "New"),
        ("project:third", "Third"),
    ] {
        journal
            .append(
                "node.upsert",
                &json!({"id": id, "type":"Project", "label":label, "provenance":"fixture"}),
            )
            .expect("project node");
    }
    drop(journal);
    let workspace = tempfile::tempdir().expect("workspace");
    let archive = workspace.path().join("Ledger Project");
    let source = workspace.path().join("reference.txt");
    std::fs::write(&source, b"reference bytes").expect("source");
    let kb_root = kb.path().to_string_lossy().into_owned();
    let archive_root = archive.to_string_lossy().into_owned();
    let source_path = source.to_string_lossy().into_owned();

    let init = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "artifact",
            "ledger",
            "init",
            "--archive-root",
            &archive_root,
            "--project-id",
            "project:archive",
            "--title",
            "Ledger Project",
        ])
        .output()
        .expect("init process");
    assert!(init.status.success());
    let init_json: serde_json::Value = serde_json::from_slice(&init.stdout).expect("init json");
    assert_eq!(init_json["status"], "initialized");
    let add = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "--format",
            "json",
            "artifact",
            "ledger",
            "add",
            "--archive-root",
            &archive_root,
            "--file",
            &source_path,
            "--source-kind",
            "document",
            "--role",
            "reference",
            "--stage",
            "verification",
            "--owner-project",
            "project:old",
            "--relation",
            "reference",
            "--document-id",
            "document:reference",
            "--provenance",
            "fixture reference",
        ])
        .output()
        .expect("add process");
    assert!(add.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&add.stdout).expect("add json");
    let receipt_id = receipt["receipt"]["id"].as_str().expect("receipt id");

    let old_query = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "--format",
            "json",
            "query",
            "--q",
            "reference.txt",
        ])
        .output()
        .expect("old query");
    let old_text = String::from_utf8(old_query.stdout).expect("utf8");
    assert!(old_text.contains("reference.txt"));
    let before = std::fs::read_to_string(kb.path().join(".innen/journal.jsonl")).expect("journal");
    assert!(before.contains("REFERENCES_PROJECT"));

    let event = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "artifact",
            "ledger",
            "event",
            "--archive-root",
            &archive_root,
            "--project-id",
            "project:archive",
            "--title",
            "Ledger Project",
            "--kind",
            "reclassify",
            "--receipt-id",
            receipt_id,
            "--new-owner-project",
            "project:new",
            "--new-relation",
            "reference",
            "--provenance",
            "owner correction",
        ])
        .output()
        .expect("event process");
    assert!(event.status.success());
    let event_json: serde_json::Value = serde_json::from_slice(&event.stdout).expect("event json");
    assert_eq!(event_json["event"], "reclassify");
    Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "artifact",
            "ledger",
            "event",
            "--archive-root",
            &archive_root,
            "--project-id",
            "project:archive",
            "--title",
            "Ledger Project",
            "--kind",
            "reclassify",
            "--receipt-id",
            receipt_id,
            "--new-owner-project",
            "project:third",
            "--new-relation",
            "reference",
            "--provenance",
            "second owner correction",
        ])
        .assert()
        .success();
    let new_query = Command::cargo_bin("innen")
        .expect("binary")
        .args([
            "--root",
            &kb_root,
            "--format",
            "json",
            "query",
            "--q",
            "reference.txt",
        ])
        .output()
        .expect("new query");
    let new_text = String::from_utf8(new_query.stdout).expect("utf8");
    assert!(new_text.contains("reference.txt"));
    let after = std::fs::read_to_string(kb.path().join(".innen/journal.jsonl")).expect("journal");
    assert!(after.contains("owner correction"));
    assert!(after.contains("second owner correction"));
    let events = after
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    let graph = materialize(&events, None, true);
    let revision = format!("artifact-revision:{receipt_id}");
    assert!(graph
        .nodes
        .contains_key(&format!("artifact-ledger-event:{receipt_id}:1")));
    assert!(graph
        .nodes
        .contains_key(&format!("artifact-ledger-event:{receipt_id}:2")));
    let active = graph
        .edges
        .iter()
        .filter(|edge| !edge.retracted && edge.from == revision)
        .map(|edge| (edge.edge.to_string(), edge.to.clone()))
        .collect::<Vec<_>>();
    assert!(active.contains(&("REFERENCES_PROJECT".into(), "project:third".into())));
    assert!(!active
        .iter()
        .any(|(_, to)| to == "project:old" || to == "project:new"));
}

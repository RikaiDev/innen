use innen_core::artifact::ledger::*;
use std::fs;
use tempfile::TempDir;

fn ledger(td: &TempDir, title: &str) -> Ledger {
    Ledger::init(InitOptions {
        root: td.path().join(title),
        project_id: "project-a".into(),
        title: title.into(),
        downloads_root: Some(td.path().join("Downloads")),
        categories: vec!["evidence".into()],
    })
    .unwrap()
}
fn add(l: &Ledger, p: std::path::PathBuf, stage: LifecycleStage) -> Receipt {
    l.add(AddOptions {
        path: p,
        document_id: None,
        source_kind: SourceKind::Document,
        role: "source".into(),
        stage,
        owner_project: "project-a".into(),
        relation: "belongs_to".into(),
        document_date: None,
        authority: None,
        evidence_status: None,
        provenance: Some("fixture".into()),
        approval_valid_from: None,
        approval_valid_until: None,
        downloads_root: None,
    })
    .unwrap()
}

#[test]
fn exact_title_and_no_copy_registration() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "正式專案");
    let src = td.path().join("source.txt");
    fs::write(&src, b"abc").unwrap();
    let r = add(&l, src.clone(), LifecycleStage::Authoring);
    assert_eq!(l.root().file_name().unwrap(), "正式專案");
    assert!(!fs::read(l.root().join(".index.json")).unwrap().is_empty());
    assert!(!l.root().join("檔案索引.json").exists());
    assert_eq!(r.bytes, 3);
    assert!(!l.root().join("source.txt").exists());
}

#[test]
fn drift_and_explicit_supersession_are_reported() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "專案");
    let p = td.path().join("a");
    fs::write(&p, b"one").unwrap();
    let old = add(&l, p.clone(), LifecycleStage::Verification);
    fs::write(&p, b"two").unwrap();
    assert_eq!(l.check().unwrap().changed.len(), 1);
    fs::write(&p, b"one").unwrap();
    let new = add(&l, p, LifecycleStage::Delivery);
    l.event(LedgerEvent::Supersede {
        receipt_id: new.id.clone(),
        superseded_receipt_id: old.id.clone(),
        event_date: Some("2026-09-06".into()),
    })
    .unwrap();
    let c = l.check().unwrap();
    assert!(c.superseded.contains(&old.id));
}

#[test]
fn relocation_requires_verified_hash() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "專案");
    let p = td.path().join("a");
    let q = td.path().join("b");
    fs::write(&p, b"same").unwrap();
    fs::write(&q, b"other").unwrap();
    let r = add(&l, p, LifecycleStage::Delivery);
    assert!(matches!(
        l.event(LedgerEvent::Relocate {
            receipt_id: r.id.clone(),
            new_path: q,
            expected_sha256: r.sha256.clone(),
            event_date: None
        }),
        Err(LedgerError::HashMismatch { .. })
    ));
}

#[test]
fn references_keep_owner_identity_separate() {
    let td = tempfile::tempdir().unwrap();
    let a = ledger(&td, "A");
    let b = Ledger::init(InitOptions {
        root: td.path().join("B"),
        project_id: "project-b".into(),
        title: "B".into(),
        downloads_root: Some(td.path().join("Downloads")),
        categories: vec![],
    })
    .unwrap();
    let p = td.path().join("shared");
    fs::write(&p, b"x").unwrap();
    let r = a
        .add(AddOptions {
            path: p,
            document_id: None,
            source_kind: SourceKind::Other,
            role: "reference".into(),
            stage: LifecycleStage::Intake,
            owner_project: "project-b".into(),
            relation: "reference".into(),
            document_date: None,
            authority: None,
            evidence_status: None,
            provenance: Some("fixture".into()),
            approval_valid_from: None,
            approval_valid_until: None,
            downloads_root: None,
        })
        .unwrap();
    assert_eq!(r.owner_project, "project-b");
    assert_eq!(b.check().unwrap().active, 0);
}

#[test]
fn unknown_date_and_download_boundary() {
    let td = tempfile::tempdir().unwrap();
    let downloads = td.path().join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    let l = ledger(&td, "專案");
    let p = downloads.join("x");
    fs::write(&p, b"x").unwrap();
    let r = add(&l, p.clone(), LifecycleStage::Intake);
    assert!(r.document_date.is_none());
    assert!(matches!(
        l.add(AddOptions {
            path: p,
            document_id: None,
            source_kind: SourceKind::Document,
            role: "x".into(),
            stage: LifecycleStage::Delivery,
            owner_project: "project-a".into(),
            relation: "belongs_to".into(),
            document_date: None,
            authority: None,
            evidence_status: None,
            provenance: Some("fixture".into()),
            approval_valid_from: None,
            approval_valid_until: None,
            downloads_root: None
        }),
        Err(LedgerError::Downloads(_))
    ));
}

#[test]
fn unrelated_preexisting_index_is_preserved() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().join("專案");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("檔案索引.json"), b"user data").unwrap();
    assert!(matches!(
        Ledger::init(InitOptions {
            root: root.clone(),
            project_id: "p".into(),
            title: "專案".into(),
            downloads_root: Some(td.path().join("Downloads")),
            categories: vec![]
        }),
        Err(LedgerError::Conflict(_))
    ));
    assert_eq!(fs::read(root.join("檔案索引.json")).unwrap(), b"user data");
}

#[test]
fn owned_legacy_index_remains_the_projection_path() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "舊專案");
    let hidden = l.root().join(".index.json");
    let legacy = l.root().join("檔案索引.json");
    fs::rename(&hidden, &legacy).unwrap();
    let reopened = ledger(&td, "舊專案");
    assert!(legacy.exists());
    assert!(!hidden.exists());
    assert!(fs::read_to_string(&legacy)
        .unwrap()
        .contains("innen-project-ledger-v1"));
    drop(reopened);
}

#[test]
fn reclassification_preserves_original_receipt_and_records_correction() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "專案");
    let p = td.path().join("source.txt");
    fs::write(&p, b"source").unwrap();
    let r = add(&l, p, LifecycleStage::Verification);

    l.event(LedgerEvent::Reclassify {
        receipt_id: r.id.clone(),
        new_owner_project: Some("project-b".into()),
        new_relation: "reference".into(),
        provenance: "IRB review correction 2026-09-06".into(),
        event_date: Some("2026-09-06".into()),
    })
    .unwrap();

    let events = fs::read_to_string(l.root().join(".innen-ledger/events.jsonl")).unwrap();
    assert!(events.contains("\"owner_project\":\"project-a\""));
    assert!(events.contains("\"new_owner_project\":\"project-b\""));
    assert!(events.contains("IRB review correction 2026-09-06"));
    let timeline = fs::read_to_string(l.root().join("專案時間線.md")).unwrap();
    assert!(timeline.contains("新歸屬=project-b"));
    assert!(timeline.contains("新關係=reference"));
    assert!(timeline.contains("更正依據=IRB review correction 2026-09-06"));
}

#[test]
fn belongs_to_reclassification_cannot_cross_archive_owner() {
    let td = tempfile::tempdir().unwrap();
    let l = ledger(&td, "專案");
    let p = td.path().join("source.txt");
    fs::write(&p, b"source").unwrap();
    let r = add(&l, p, LifecycleStage::Verification);

    assert!(matches!(
        l.event(LedgerEvent::Reclassify {
            receipt_id: r.id,
            new_owner_project: Some("project-b".into()),
            new_relation: "belongs_to".into(),
            provenance: "bad correction".into(),
            event_date: None,
        }),
        Err(LedgerError::Invalid(message)) if message.contains("archive project")
    ));
}

#[test]
fn receipt_ids_are_namespaced_by_archive_identity() {
    let td = tempfile::tempdir().unwrap();
    let source = td.path().join("same.txt");
    fs::write(&source, b"same bytes").unwrap();
    let a = Ledger::init(InitOptions {
        root: td.path().join("A"),
        project_id: "project-a".into(),
        title: "A".into(),
        downloads_root: Some(td.path().join("Downloads")),
        categories: vec![],
    })
    .unwrap();
    let b = Ledger::init(InitOptions {
        root: td.path().join("B"),
        project_id: "project-b".into(),
        title: "B".into(),
        downloads_root: Some(td.path().join("Downloads")),
        categories: vec![],
    })
    .unwrap();
    let ra = add(&a, source.clone(), LifecycleStage::Verification);
    let rb = b
        .add(AddOptions {
            path: source,
            document_id: None,
            source_kind: SourceKind::Document,
            role: "source".into(),
            stage: LifecycleStage::Verification,
            owner_project: "project-b".into(),
            relation: "belongs_to".into(),
            document_date: None,
            authority: None,
            evidence_status: None,
            provenance: Some("fixture".into()),
            approval_valid_from: None,
            approval_valid_until: None,
            downloads_root: None,
        })
        .unwrap();
    assert_ne!(ra.id, rb.id);
}

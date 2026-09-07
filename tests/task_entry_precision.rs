use innen_core::journal::Journal;
use innen_core::task_entry::{task_entry, TaskEntryOptions};
use serde_json::json;

fn node(journal: &Journal, id: &str, kind: &str, label: &str, body: &str) {
    journal
        .append(
            "node.upsert",
            &json!({"id": id, "type": kind, "label": label, "body": body, "provenance": "fixture"}),
        )
        .expect("append node");
}

#[test]
fn natural_asset_request_keeps_named_entity_and_rejects_body_only_collision() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    node(
        &journal,
        "artifact:weemed",
        "Artifact",
        "weemed-yi-jian.png",
        "WeeMed 標準字參考圖",
    );
    node(
        &journal,
        "project:pcne",
        "Project",
        "PCNe clinical workspace",
        "標準字與醫囑研究",
    );
    node(
        &journal,
        "artifact:fansee",
        "Artifact",
        "FANSEE R8 標誌與標準字定稿",
        "brand review",
    );
    node(
        &journal,
        "project:semantic",
        "Project",
        "建立台語口語→標準漢字概念 semantic benchmark",
        "language benchmark",
    );
    node(
        &journal,
        "project:weemed-pcne",
        "Project",
        "workspace:weemed-ai/pcne",
        "clinical workspace",
    );
    drop(journal);

    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "請將 weemed 標準字的中文重新造字，粗細要一致".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");
    let rows = out["rows"].as_array().expect("compact rows");
    assert!(rows.iter().any(|row| row[0] == "artifact:weemed"));
    assert!(!rows.iter().any(|row| row[0] == "project:pcne"));
    assert!(!rows.iter().any(|row| {
        matches!(
            row[0].as_str(),
            Some("artifact:fansee") | Some("project:semantic") | Some("project:weemed-pcne")
        )
    }));
    assert_eq!(out["resolution"], "missing");
    assert!(out["missing_evidence"]
        .as_array()
        .expect("missing evidence")
        .iter()
        .any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("approved editable baseline"))));
}

#[test]
fn hyphenated_filename_is_a_literal_candidate() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    node(
        &journal,
        "artifact:weemed",
        "Artifact",
        "weemed-yi-jian.png",
        "reference",
    );
    drop(journal);
    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "weemed-yi-jian.png".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");
    assert!(out["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .any(|row| row[0] == "artifact:weemed"));
}

#[test]
fn approval_requires_verified_location_and_never_picks_between_baselines() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    journal
        .append(
            "node.upsert",
            &json!({"id":"artifact:one","type":"Artifact","label":"weemed-master.svg","path":"/missing/master.svg","status":"approved","provenance":"fixture"}),
        )
        .expect("append unverified baseline");
    drop(journal);
    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "weemed redraw vector".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");
    assert_eq!(out["resolution"], "missing");

    let journal = Journal::open(kb.path()).expect("reopen journal");
    for id in ["artifact:two", "artifact:three"] {
        journal
            .append(
                "node.upsert",
                &json!({"id":id,"type":"Artifact","label":format!("weemed-{id}.svg"),"path":format!("/verified/{id}.svg"),"status":"approved","location_verified":true,"provenance":"fixture"}),
            )
            .expect("append verified baseline");
    }
    drop(journal);
    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "weemed redraw vector".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");
    assert_eq!(out["resolution"], "ambiguous");
    assert!(out["baseline"].is_null());
}

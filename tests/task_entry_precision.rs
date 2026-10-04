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
fn topic_query_requires_conjunctive_body_match_instead_of_one_generic_anchor() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    node(
        &journal,
        "artifact:relevant",
        "Artifact",
        "2026-08-26-browser-history.md",
        "Browser Agent Windows portable bundle is ready for the station.",
    );
    node(
        &journal,
        "artifact:generic-agent",
        "Artifact",
        "agent handbook",
        "Generic agent operating notes without the requested topic.",
    );
    node(
        &journal,
        "artifact:partial",
        "Artifact",
        "browser-agent notes",
        "Browser Agent Linux notes only.",
    );
    drop(journal);

    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "browser agent Windows".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");
    let rows = out["rows"].as_array().expect("compact rows");
    assert!(rows.iter().any(|row| row[0] == "artifact:relevant"));
    assert!(!rows.iter().any(|row| row[0] == "artifact:generic-agent"));
    assert!(!rows.iter().any(|row| row[0] == "artifact:partial"));
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

#[test]
fn exact_node_id_is_retrievable_by_its_own_id() {
    // An id is the handle every other command accepts, so a query that *is*
    // an id must return that node. It used to score 0: the all-terms gate
    // splits `harvest-gap:innen-stop-hook-stale-pending-count` into parts and
    // then looks for those parts in label/body, where an id never appears.
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    node(
        &journal,
        "harvest-gap:innen-stop-hook-stale-pending-count",
        "harvest-gap",
        "innen stop-hook pending 數字與實際收件匣脫節",
        "pending 快照噪音根因：Antigravity hooks.json 未帶 --cwd",
    );
    node(
        &journal,
        "task:other",
        "Task",
        "unrelated label",
        "unrelated body text",
    );
    drop(journal);

    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "harvest-gap:innen-stop-hook-stale-pending-count".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");

    // `rows` are positional arrays ordered by `fields`: id, kind, label,
    // score, status, why.
    let fields: Vec<&str> = out["fields"]
        .as_array()
        .expect("fields array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let col = |name: &str| {
        fields
            .iter()
            .position(|f| *f == name)
            .expect("field column")
    };
    let (id_c, score_c, why_c) = (col("id"), col("score"), col("why"));
    let rows = out["rows"].as_array().expect("rows array");
    let first = rows[0].as_array().expect("row array");
    assert_eq!(
        first[id_c].as_str(),
        Some("harvest-gap:innen-stop-hook-stale-pending-count"),
        "the queried id must rank first: {out}"
    );
    assert_eq!(first[why_c].as_str(), Some("literal_match"), "why: {out}");
    assert_eq!(first[score_c].as_f64(), Some(1.0), "score: {out}");
}

#[test]
fn partial_id_still_obeys_the_all_terms_gate() {
    // The exemption is only for a full id. A partial id must not become a
    // back door around the precision rule.
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    node(&journal, "a:1", "Task", "shared label", "unrelated prose");
    node(
        &journal,
        "b:2",
        "Task",
        "shared label",
        "prose containing needle here",
    );
    drop(journal);

    let out = task_entry(
        kb.path(),
        &TaskEntryOptions {
            q: "shared needle".to_string(),
            ..TaskEntryOptions::default()
        },
    )
    .expect("task entry succeeds");

    let ids: Vec<&str> = out["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .filter_map(|r| r.as_array())
        .filter_map(|r| r.first().and_then(|v| v.as_str()))
        .collect();
    assert!(
        !ids.contains(&"a:1"),
        "a node matching only one term must stay out: {ids:?}"
    );
}

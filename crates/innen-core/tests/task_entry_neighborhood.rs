//! Task-context retrieval must not answer "nothing" when part of a long
//! request matches, and must show what already exists for a matched project.

use innen_core::task_entry::{task_entry, TaskEntryOptions};
use serde_json::{json, Value};

fn root_with(events: &[(&str, Value)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = innen_core::journal::Journal::open(dir.path()).expect("journal open");
    for (op, payload) in events {
        journal.append(op, payload).expect("append");
    }
    dir
}

fn fixture() -> tempfile::TempDir {
    root_with(&[
        (
            "node.upsert",
            json!({"id": "project:ntuh", "type": "Project", "label": "台大雲林專案"}),
        ),
        (
            "node.upsert",
            json!({"id": "artifact:deck1", "type": "Artifact", "label": "資安審查簡報第一版.pptx"}),
        ),
        (
            "node.upsert",
            json!({"id": "artifact:plan", "type": "Artifact", "label": "研究計畫申請書.docx"}),
        ),
        (
            "edge.assert",
            json!({"from": "artifact:deck1", "type": "BELONGS_TO", "to": "project:ntuh"}),
        ),
        (
            "edge.assert",
            json!({"from": "artifact:plan", "type": "BELONGS_TO", "to": "project:ntuh"}),
        ),
    ])
}

fn run(root: &std::path::Path, q: &str) -> Value {
    task_entry(
        root,
        &TaskEntryOptions {
            q: q.to_string(),
            ..Default::default()
        },
    )
    .expect("query")
}

#[test]
fn long_request_falls_back_to_per_term_matches_and_names_the_misses() {
    let dir = fixture();
    let out = run(dir.path(), "資委會 簡報 沒過 台大雲林");
    assert_eq!(out["resolution"], "partial");
    let missing = out["missing_evidence"].to_string();
    assert!(
        missing.contains("資委會") && missing.contains("沒過"),
        "{missing}"
    );
    assert!(out["rows"].to_string().contains("project:ntuh"));
}

#[test]
fn matched_project_is_linked_and_lists_its_siblings() {
    let dir = fixture();
    let out = run(dir.path(), "台大雲林專案");
    assert_eq!(out["linked_projects"], json!(["project:ntuh"]));
    assert_eq!(out["neighborhood_total"], 2);
    let near = out["neighborhood"].to_string();
    assert!(
        near.contains("資安審查簡報第一版") && near.contains("研究計畫申請書"),
        "{near}"
    );
}

#[test]
fn nothing_matching_stays_missing() {
    let dir = fixture();
    let out = run(dir.path(), "完全無關 另一個詞");
    assert_eq!(out["resolution"], "missing");
}

#[test]
fn request_that_already_hit_files_skips_the_sibling_list() {
    let dir = fixture();
    let out = run(dir.path(), "資安審查簡報第一版");
    assert_eq!(out["linked_projects"], json!(["project:ntuh"]));
    assert_eq!(out["neighborhood"], json!([]));
}

#[test]
fn project_constraints_are_listed_with_their_source_and_not_as_siblings() {
    let dir = root_with(&[
        (
            "node.upsert",
            json!({"id": "project:ntuh", "type": "Project", "label": "台大雲林專案"}),
        ),
        (
            "node.upsert",
            json!({"id": "artifact:deck1", "type": "Artifact", "label": "第一版簡報.pptx"}),
        ),
        (
            "node.upsert",
            json!({"id": "constraint:1", "type": "Constraint", "label": "只談台大自己的據點",
            "body": "資委會是針對台大報告", "provenance": "agy:ee6438d3:L510"}),
        ),
        (
            "edge.assert",
            json!({"from": "artifact:deck1", "type": "BELONGS_TO", "to": "project:ntuh"}),
        ),
        (
            "edge.assert",
            json!({"from": "constraint:1", "type": "BELONGS_TO", "to": "project:ntuh"}),
        ),
    ]);
    let out = run(dir.path(), "台大雲林專案");
    assert_eq!(out["constraints_total"], 1);
    let c = out["constraints"].to_string();
    assert!(
        c.contains("資委會是針對台大報告") && c.contains("agy:ee6438d3:L510"),
        "{c}"
    );
    assert!(!out["neighborhood"].to_string().contains("constraint:1"));
    assert!(innen_core::task_entry::render(&out).contains("Binding constraints"));
}

fn many_constraints() -> tempfile::TempDir {
    let ev: Vec<(&str, Value)> = vec![(
        "node.upsert",
        json!({"id": "project:ntuh", "type": "Project", "label": "台大雲林專案"}),
    )];
    let mut owned: Vec<(String, Value)> = Vec::new();
    for i in 0..8 {
        let body = if i == 7 {
            "只談台大自己的據點".to_string()
        } else {
            format!("無關的要求 {i} {}", "長".repeat(300))
        };
        owned.push((
            "node.upsert".into(),
            json!({"id": format!("constraint:{i}"), "type": "Constraint", "label": format!("c{i}"),
                   "body": body, "provenance": format!("s#L{i}")}),
        ));
        owned.push((
            "edge.assert".into(),
            json!({"from": format!("constraint:{i}"), "type": "BELONGS_TO", "to": "project:ntuh", "provenance": "s"}),
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let journal = innen_core::journal::Journal::open(dir.path()).unwrap();
    for (op, p) in &ev {
        journal.append(op, p).unwrap();
    }
    for (op, p) in &owned {
        journal.append(op, p).unwrap();
    }
    dir
}

#[test]
fn constraints_matching_the_query_come_first() {
    let dir = many_constraints();
    let out = run(dir.path(), "台大雲林專案 台大自己");
    assert_eq!(out["constraints"][0]["id"], "constraint:7");
}

#[test]
fn a_budget_drops_detail_in_order_and_says_so() {
    let dir = many_constraints();
    let big = run(dir.path(), "台大雲林專案");
    let size = |v: &Value| v.to_string().chars().count();
    let budget = size(&big) / 3;
    let out = task_entry(
        dir.path(),
        &TaskEntryOptions {
            q: "台大雲林專案".into(),
            budget: Some(budget),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        out["budget"]["cut"].as_array().unwrap().len() >= 2,
        "{}",
        out["budget"]
    );
    assert!(out["constraints"].as_array().unwrap().len() <= 4);
    assert_eq!(out["resolution"], big["resolution"]);
    assert_eq!(out["evidence_expansion"], big["evidence_expansion"]);
    assert!(size(&out) < size(&big));
    // a request that already fits is returned untouched
    let roomy = task_entry(
        dir.path(),
        &TaskEntryOptions {
            q: "台大雲林專案".into(),
            budget: Some(size(&big) + 10),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(roomy.get("budget").is_none());
}

#[test]
fn a_term_found_in_a_recorded_constraint_is_not_reported_missing() {
    let dir = many_constraints();
    let out = run(dir.path(), "台大雲林專案 台大自己 從未出現過的詞");
    let missing = out["missing_evidence"].to_string();
    assert!(missing.contains("從未出現過的詞"), "{missing}");
    assert!(!missing.contains("台大自己"), "{missing}");
}

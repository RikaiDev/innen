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

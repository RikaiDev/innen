use assert_cmd::Command;
use innen_core::artifact;
use innen_core::graph::{materialize, EdgeType, Endpoint, NodeType};
use innen_core::journal::Journal;
use innen_core::query::{query, QueryParams};
use serde_json::json;

fn append_node(journal: &Journal, id: &str, kind: &str, label: &str) {
    journal
        .append(
            "node.upsert",
            &json!({"id": id, "type": kind, "label": label, "provenance": "test fixture"}),
        )
        .expect("append node");
}

#[test]
fn artifact_belongs_to_project_is_shared_by_cli_artifact_add_and_query() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "project:one", "Project", "One");
    append_node(&journal, "artifact:manual", "Artifact", "manual.txt");

    let source = kb.path().join("manual.txt");
    std::fs::write(&source, b"manual evidence").expect("write source");
    drop(journal);
    let receipt = artifact::add(kb.path(), &source, Some("project:one"))
        .expect("artifact add --project records membership");

    let root = kb.path().to_string_lossy().into_owned();
    Command::cargo_bin("innen")
        .expect("innen binary")
        .args([
            "--root",
            &root,
            "graph",
            "relate",
            "--from",
            &format!("artifact:{}", &receipt.sha256[..8]),
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:one",
        ])
        .assert()
        .success();

    let journal = Journal::open(kb.path()).expect("reopen journal");
    let entries = journal.read_all().expect("read journal");
    let events: Vec<_> = entries
        .iter()
        .map(|e| json!({"op": e.op, "payload": e.payload, "observed_utc": e.observed_utc}))
        .collect();
    let materialized = materialize(&events, None, true);
    assert!(innen_core::graph::validate(
        &NodeType::Artifact,
        &EdgeType::BelongsTo,
        &Endpoint::Node(NodeType::Project),
        Some("test fixture"),
    )
    .is_ok());
    assert!(materialized.edges.iter().any(|edge| {
        edge.from == format!("artifact:{}", &receipt.sha256[..8])
            && edge.to == "project:one"
            && edge.edge == EdgeType::BelongsTo
    }));

    let out = query(
        kb.path(),
        &QueryParams {
            q: "manual.txt".to_string(),
            as_of: None,
            limit: 20,
            include_expired: true,
        },
    )
    .expect("query succeeds");
    assert!(out.hits.iter().any(|hit| hit.node_id == "project:one"));
}

//! `project --view full` is the handoff page for a project: it must say who the
//! project is, list every kind of member, and say when it was rendered.
//!
//! Three properties come from how the page was actually consumed. On 2026-09-02
//! the PCNe monthly report spec listed the generated page as `sources[0]`, ahead
//! of the research minutes, with `forbidden_cross_project_sources` naming
//! `project:itri-icope` — so identity must be unambiguous. Membership must be
//! bidirectional, because `HAS_WORKSTREAM` runs project → workstream while
//! `IMPLEMENTS`/`SUBJECT_OF`/`INFORMS` run the other way. And grouping must key
//! on the id prefix, because in practice every `workstream:` node is typed
//! `Task` and every `workspace:` node is typed `Project`.

use assert_cmd::Command;

use innen_core::journal::Journal;

fn seed(root: &std::path::Path, nodes: &[serde_json::Value], edges: &[serde_json::Value]) {
    let j = Journal::open(root).unwrap();
    for node in nodes {
        j.append("node.upsert", node).unwrap();
    }
    for edge in edges {
        j.append("edge.assert", edge).unwrap();
    }
}

fn page(root: &std::path::Path, id: &str) -> String {
    let out = Command::cargo_bin("innen")
        .unwrap()
        .arg("--root")
        .arg(root)
        .args(["project", id, "--view", "full", "--format", "human"])
        .assert()
        .success()
        .get_output()
        .stdout
        .to_vec();
    String::from_utf8(out).unwrap()
}

/// Identity: an organization-qualified title and a slug a reader can pass back.
#[test]
fn page_identifies_the_project_and_when_it_was_rendered() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[serde_json::json!({
            "id": "project:icope", "type": "Project",
            "label": "ICOPE 委託開發",
            "organization": "工業技術研究院（ITRI）",
            "status": "active",
        })],
        &[],
    );

    let out = page(root, "icope");
    assert!(
        out.starts_with("# 工業技術研究院（ITRI）｜ICOPE 委託開發\n"),
        "title must carry the organization: {out}"
    );
    assert!(out.contains("- Project ID: `icope`"), "{out}");
    assert!(out.contains("- Status: `active`"), "{out}");
    assert!(
        out.contains("- Graph generated: `"),
        "a reader must be able to tell the page's age: {out}"
    );
}

/// Bidirectional membership: `HAS_WORKSTREAM` points out of the project, and
/// `IMPLEMENTS`/`SUBJECT_OF` point in. A previous version matched only
/// `edge.to == project`, so all three kinds were invisible.
#[test]
fn page_lists_members_on_either_side_of_the_project_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[
            serde_json::json!({"id": "project:p", "type": "Project", "label": "P"}),
            serde_json::json!({"id": "workstream:p:one", "type": "Task", "label": "ws one"}),
            serde_json::json!({"id": "workspace:acme/repo", "type": "Project", "label": "repo"}),
            serde_json::json!({"id": "product:cap", "type": "Product", "label": "cap"}),
        ],
        &[
            serde_json::json!({"from": "project:p", "type": "HAS_WORKSTREAM", "to": "workstream:p:one"}),
            serde_json::json!({"from": "workspace:acme/repo", "type": "IMPLEMENTS", "to": "project:p"}),
            serde_json::json!({"from": "product:cap", "type": "SUBJECT_OF", "to": "project:p"}),
        ],
    );

    let out = page(root, "p");
    assert!(out.contains("## Workstreams"), "{out}");
    assert!(out.contains("ws one"), "outbound workstream missing: {out}");
    assert!(out.contains("`HAS_WORKSTREAM`"), "edge type missing: {out}");
    assert!(out.contains("## Codebases / workspaces"), "{out}");
    assert!(out.contains("repo"), "inbound workspace missing: {out}");
    assert!(out.contains("`IMPLEMENTS`"), "{out}");
    assert!(out.contains("## Products / capabilities"), "{out}");
    assert!(out.contains("cap"), "inbound product missing: {out}");
}

/// Grouping keys on the id prefix, not the node type. The fixture encodes the
/// real mismatch: a workstream typed `Task`, a workspace typed `Project`.
#[test]
fn grouping_follows_the_id_prefix_not_the_node_type() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[
            serde_json::json!({"id": "project:p", "type": "Project", "label": "P"}),
            serde_json::json!({"id": "workstream:p:one", "type": "Task", "label": "ws one"}),
            serde_json::json!({"id": "workspace:acme/repo", "type": "Project", "label": "repo"}),
            serde_json::json!({"id": "task:p:real", "type": "Task", "label": "real task"}),
        ],
        &[
            serde_json::json!({"from": "project:p", "type": "HAS_WORKSTREAM", "to": "workstream:p:one"}),
            serde_json::json!({"from": "workspace:acme/repo", "type": "IMPLEMENTS", "to": "project:p"}),
            serde_json::json!({"from": "task:p:real", "type": "BELONGS_TO", "to": "project:p"}),
        ],
    );

    let out = page(root, "p");
    let workstreams = out
        .split("## Workstreams")
        .nth(1)
        .unwrap()
        .split("## ")
        .next()
        .unwrap();
    let tasks = out
        .split("## Tasks / blockers")
        .nth(1)
        .unwrap()
        .split("## ")
        .next()
        .unwrap();
    assert!(workstreams.contains("ws one"), "{workstreams}");
    assert!(
        !workstreams.contains("real task"),
        "a task must not land under Workstreams: {workstreams}"
    );
    assert!(tasks.contains("real task"), "{tasks}");
    assert!(
        !tasks.contains("ws one"),
        "a workstream must not land under Tasks: {tasks}"
    );
}

/// Per-row context, so a reader cannot mistake the page for narrative prose.
#[test]
fn rows_carry_status_edge_and_date() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[
            serde_json::json!({"id": "project:p", "type": "Project", "label": "P"}),
            serde_json::json!({
                "id": "task:p:a", "type": "Task", "label": "do the thing",
                "status": "open", "date": "2026-09-02",
                "summary": "what it unblocks",
            }),
            serde_json::json!({
                "id": "artifact:p:a", "type": "Artifact", "label": "report.docx",
                "status": "recovered-final", "path": "/tmp/report.docx",
            }),
        ],
        &[
            serde_json::json!({"from": "task:p:a", "type": "BELONGS_TO", "to": "project:p"}),
            serde_json::json!({"from": "artifact:p:a", "type": "BELONGS_TO", "to": "project:p"}),
        ],
    );

    let out = page(root, "p");
    assert!(
        out.contains("- 2026-09-02｜do the thing [open] `BELONGS_TO`：what it unblocks"),
        "{out}"
    );
    assert!(
        out.contains("- report.docx [recovered-final] `BELONGS_TO` — `/tmp/report.docx`"),
        "{out}"
    );
}

/// `organization` and `status` are written in one place only —
/// `04-index/projects/registry.json` — and graph nodes carry them only for the
/// projects that happened to be seeded from it. A thin node must still render an
/// organization-qualified title, otherwise a reader cannot tell two similarly
/// named projects apart.
#[test]
fn identity_falls_back_to_the_registry_for_a_thin_node() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[serde_json::json!({"id": "project:thin", "type": "Project", "label": "Thin"})],
        &[],
    );
    std::fs::create_dir_all(root.join("04-index/projects")).unwrap();
    std::fs::write(
        root.join("04-index/projects/registry.json"),
        r#"{"projects":[{"id":"thin","organization":"某機關","status":"needs-ingest"}]}"#,
    )
    .unwrap();

    let out = page(root, "thin");
    assert!(
        out.starts_with("# 某機關｜Thin\n"),
        "organization must come from the registry: {out}"
    );
    assert!(out.contains("- Status: `needs-ingest`"), "{out}");
}

/// A missing or unparseable registry must not break the page.
#[test]
fn page_renders_without_a_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[serde_json::json!({"id": "project:solo", "type": "Project", "label": "Solo"})],
        &[],
    );

    let out = page(root, "solo");
    assert!(out.starts_with("# Solo\n"), "{out}");
    assert!(!out.contains("- Status:"), "no status to show: {out}");
}
/// Ordering has to be stable across runs or the page cannot be diffed.
#[test]
fn undated_rows_come_first_then_dates_ascending() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed(
        root,
        &[
            serde_json::json!({"id": "project:p", "type": "Project", "label": "P"}),
            serde_json::json!({"id": "task:p:late", "type": "Task", "label": "late", "date": "2026-09-03"}),
            serde_json::json!({"id": "task:p:early", "type": "Task", "label": "early", "date": "2026-08-01"}),
            serde_json::json!({"id": "task:p:none", "type": "Task", "label": "undated"}),
        ],
        &[
            serde_json::json!({"from": "task:p:late", "type": "BELONGS_TO", "to": "project:p"}),
            serde_json::json!({"from": "task:p:early", "type": "BELONGS_TO", "to": "project:p"}),
            serde_json::json!({"from": "task:p:none", "type": "BELONGS_TO", "to": "project:p"}),
        ],
    );

    let out = page(root, "p");
    let tasks = out
        .split("## Tasks / blockers")
        .nth(1)
        .unwrap()
        .split("## ")
        .next()
        .unwrap();
    let order: Vec<usize> = ["undated", "early", "late"]
        .into_iter()
        .map(|needle| {
            tasks
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} missing: {tasks}"))
        })
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "expected undated, then 2026-08-01, then 2026-09-03: {tasks}"
    );
}

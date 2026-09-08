use innen_core::graph::{materialize, EdgeType};
use innen_core::journal::Journal;
use innen_core::wiki_graph;
use serde_json::json;

fn write_page(root: &std::path::Path, relative: &str, sources: &str, related: &str) {
    let path = root.join("02-wiki").join(relative);
    std::fs::create_dir_all(path.parent().expect("page parent")).expect("create wiki dir");
    std::fs::write(
        path,
        format!(
            "---\ntitle: Example\ntags: [one, two]\nsources: {sources}\nrelated: {related}\n---\n# Example\n\nBody.\n"
        ),
    )
    .expect("write page");
}

fn graph(root: &std::path::Path) -> innen_core::graph::Materialized {
    let journal = Journal::open(root).expect("open journal");
    let events: Vec<_> = journal
        .read_all()
        .expect("read journal")
        .into_iter()
        .map(|event| {
            json!({
                "op": event.op,
                "payload": event.payload,
                "observed_utc": event.observed_utc,
            })
        })
        .collect();
    materialize(&events, None, true)
}

fn journal_lines(root: &std::path::Path) -> usize {
    std::fs::read_to_string(root.join(".innen/journal.jsonl"))
        .expect("journal exists")
        .lines()
        .count()
}

#[test]
fn credential_like_wiki_content_is_not_projected() {
    let root = tempfile::tempdir().unwrap();
    write_page(root.path(), "private.md", "[]", "[]");
    let path = root.path().join("02-wiki/private.md");
    let text = std::fs::read_to_string(&path).unwrap() + "\n-----BEGIN PRIVATE KEY-----\nfixture\n";
    std::fs::write(&path, text).unwrap();
    assert!(wiki_graph::sync(root.path()).is_err());
    assert!(!root.path().join(".innen/journal.jsonl").exists());
    assert!(path.exists());
}

#[test]
fn projects_wiki_sources_related_and_is_idempotent() {
    let root = tempfile::tempdir().expect("temp root");
    write_page(root.path(), "ai/target.md", "[]", "[]");
    write_page(
        root.path(),
        "ai/origin.md",
        "[\"[[target]]\", \"https://example.test/evidence\", \"conversation:codex:123e4567-e89b-12d3-a456-426614174000\"]",
        "target",
    );

    let first = wiki_graph::sync(root.path()).expect("first sync");
    assert_eq!(first.pages_scanned, 2);
    assert_eq!(first.wiki_nodes_created, 2);
    assert_eq!(first.source_nodes_created, 2);
    assert_eq!(first.edges_asserted, 4);

    let projected = graph(root.path());
    let wiki_nodes: Vec<_> = projected
        .nodes
        .values()
        .filter(|node| node["type"] == "Wiki")
        .collect();
    assert_eq!(wiki_nodes.len(), 2);
    assert!(wiki_nodes.iter().all(|node| {
        node["path"]
            .as_str()
            .expect("absolute path")
            .starts_with('/')
            && node["managed_by"] == wiki_graph::MANAGED_BY
    }));
    assert!(projected.nodes.values().any(|node| {
        node["locator"] == json!({"kind":"url", "url":"https://example.test/evidence"})
    }));
    assert!(projected.nodes.values().any(|node| {
        node["locator"]["kind"] == "conversation" && node["locator"]["source"] == "codex"
    }));

    let before = journal_lines(root.path());
    let second = wiki_graph::sync(root.path()).expect("idempotent sync");
    assert_eq!(journal_lines(root.path()), before);
    assert_eq!(second.wiki_nodes_created + second.wiki_nodes_updated, 0);
    assert_eq!(second.source_nodes_created + second.source_nodes_updated, 0);
    assert_eq!(second.edges_asserted + second.edges_retracted, 0);
}

#[test]
fn changed_sources_retract_owned_edge_and_preserve_foreign_edge() {
    let root = tempfile::tempdir().expect("temp root");
    write_page(root.path(), "page.md", "old-source.txt", "[]");
    wiki_graph::sync(root.path()).expect("initial sync");
    let before = graph(root.path());
    let wiki_id = before
        .nodes
        .iter()
        .find(|(_, node)| node["type"] == "Wiki")
        .map(|(id, _)| id.clone())
        .expect("wiki id");

    let journal = Journal::open(root.path()).expect("open journal");
    journal
        .append(
            "node.upsert",
            &json!({"id":"source:foreign", "type":"Source", "label":"foreign", "provenance":"fixture"}),
        )
        .expect("foreign node");
    journal
        .append(
            "edge.assert",
            &json!({"from":wiki_id, "type":"RELATED_TO", "to":"source:foreign", "provenance":"foreign fixture"}),
        )
        .expect("foreign edge");
    drop(journal);

    write_page(root.path(), "page.md", "new-source.txt", "[]");
    let report = wiki_graph::sync(root.path()).expect("changed sync");
    assert_eq!(report.edges_retracted, 1);
    assert_eq!(report.edges_asserted, 1);

    let after = graph(root.path());
    assert!(after.edges.iter().any(|edge| {
        !edge.retracted
            && edge.from == wiki_id
            && edge.to == "source:foreign"
            && edge.edge == EdgeType::Custom("RELATED_TO".to_string())
    }));
    let live_derived: Vec<_> = after
        .edges
        .iter()
        .filter(|edge| !edge.retracted && edge.edge == EdgeType::DerivedFrom)
        .collect();
    assert_eq!(live_derived.len(), 1);
    assert_eq!(
        after.nodes[&live_derived[0].to]["source_ref"],
        "new-source.txt"
    );
}

#[test]
fn malformed_yaml_writes_nothing() {
    let root = tempfile::tempdir().expect("temp root");
    let wiki = root.path().join("02-wiki");
    std::fs::create_dir(&wiki).expect("wiki dir");
    std::fs::write(
        wiki.join("bad.md"),
        "---\ntitle: [unterminated\ntags: []\nsources: []\nrelated: []\n---\nbody\n",
    )
    .expect("bad page");

    let error = wiki_graph::sync(root.path()).expect_err("malformed YAML rejected");
    assert!(error.to_string().contains("invalid wiki frontmatter"));
    assert!(!root.path().join(".innen/journal.jsonl").exists());
}

#[test]
fn source_targets_are_never_read_and_relative_escape_is_skipped() {
    let root = tempfile::tempdir().expect("temp root");
    write_page(
        root.path(),
        "page.md",
        "[\"missing/private/source.bin\", \"../../escape.txt\"]",
        "[]",
    );
    let report = wiki_graph::sync(root.path()).expect("reference-only sync");
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("escaping the knowledge-base root")));
    let projected = graph(root.path());
    assert!(projected.nodes.values().any(|node| {
        node["source_ref"] == "missing/private/source.bin" && node["locator"]["kind"] == "file"
    }));
    assert!(!projected
        .nodes
        .values()
        .any(|node| node["source_ref"] == "../../escape.txt"));
}

#[test]
fn deleted_page_is_marked_missing_without_deleting_its_node() {
    let root = tempfile::tempdir().expect("temp root");
    write_page(root.path(), "page.md", "evidence.txt", "[]");
    wiki_graph::sync(root.path()).expect("initial sync");
    let before = graph(root.path());
    let wiki_id = before
        .nodes
        .iter()
        .find(|(_, node)| node["type"] == "Wiki")
        .map(|(id, _)| id.clone())
        .expect("wiki id");
    std::fs::remove_file(root.path().join("02-wiki/page.md")).expect("remove page fixture");

    let report = wiki_graph::sync(root.path()).expect("missing-page sync");
    assert_eq!(report.wiki_nodes_marked_missing, 1);
    assert_eq!(report.edges_retracted, 1);
    let after = graph(root.path());
    assert_eq!(after.nodes[&wiki_id]["missing"], true);
    assert!(!after
        .edges
        .iter()
        .any(|edge| !edge.retracted && edge.from == wiki_id));
}

#[cfg(unix)]
#[test]
fn traversal_skips_symlink_that_escapes_wiki_root() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("temp root");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::create_dir(root.path().join("02-wiki")).expect("wiki dir");
    write_page(outside.path(), "outside.md", "[]", "[]");
    symlink(
        outside.path().join("02-wiki/outside.md"),
        root.path().join("02-wiki/escaped.md"),
    )
    .expect("symlink");

    let report = wiki_graph::sync(root.path()).expect("skip symlink");
    assert_eq!(report.pages_scanned, 0);
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("outside canonical wiki root")));
}

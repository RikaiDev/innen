use assert_cmd::Command;
use innen_core::journal::Journal;
use serde_json::json;

fn append_node(journal: &Journal, id: &str, kind: &str) {
    journal
        .append("node.upsert", &json!({"id": id, "type": kind, "label": id}))
        .expect("append node fixture");
}

fn relate(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("innen")
        .expect("innen binary")
        .timeout(std::time::Duration::from_secs(10))
        .args([
            "--root",
            root.to_str().expect("utf-8 root"),
            "graph",
            "relate",
        ])
        .args(args)
        .output()
        .expect("run graph relate")
}

fn journal_bytes(root: &std::path::Path) -> Vec<u8> {
    std::fs::read(root.join(".innen/journal.jsonl")).expect("read journal")
}

#[test]
fn rejects_missing_target_without_mutating_journal() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "task:one", "Task");
    let before = journal_bytes(kb.path());
    drop(journal);

    let output = relate(
        kb.path(),
        &[
            "--from",
            "task:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "projet:typo",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(journal_bytes(kb.path()), before);
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("missing graph endpoint: --to \"projet:typo\""));
}

#[test]
fn rejects_missing_source() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "project:one", "Project");
    drop(journal);

    let output = relate(
        kb.path(),
        &[
            "--from",
            "task:missing",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:one",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("missing graph endpoint: --from \"task:missing\""));
}

#[test]
fn accepts_existing_typed_endpoints() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "task:one", "Task");
    append_node(&journal, "project:one", "Project");
    drop(journal);

    let output = relate(
        kb.path(),
        &[
            "--from",
            "task:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:one",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        journal_bytes(kb.path())
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        3
    );
}

#[test]
fn accepts_legal_uri_target_without_a_node() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "artifact:one", "Artifact");
    drop(journal);

    let output = relate(
        kb.path(),
        &[
            "--from",
            "artifact:one",
            "--edge",
            "LOCATED_AT",
            "--to",
            "file:///tmp/one",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn allow_dangling_is_explicit_and_preserves_known_provenance_rules() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal = Journal::open(kb.path()).expect("open journal");
    append_node(&journal, "extension:one", "Extension");
    drop(journal);

    let missing_provenance = relate(
        kb.path(),
        &[
            "--from",
            "extension:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:later",
            "--allow-dangling",
        ],
    );
    assert!(!missing_provenance.status.success());

    let allowed = relate(
        kb.path(),
        &[
            "--from",
            "extension:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:later",
            "--allow-dangling",
            "--provenance",
            "ordered migration receipt",
        ],
    );
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}

#[test]
fn corrupt_journal_fails_without_quarantine_or_rewrite() {
    let kb = tempfile::tempdir().expect("temp kb");
    let innen = kb.path().join(".innen");
    std::fs::create_dir(&innen).expect("create .innen");
    let corrupt = b"{not-json}\n";
    std::fs::write(innen.join("journal.jsonl"), corrupt).expect("write corrupt journal");

    let output = relate(
        kb.path(),
        &[
            "--from",
            "task:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:one",
            "--allow-dangling",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(journal_bytes(kb.path()), corrupt);
    assert!(!innen.join("quarantine").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid graph journal"));
}

#[test]
fn unreadable_journal_path_fails_closed() {
    let kb = tempfile::tempdir().expect("temp kb");
    let journal_path = kb.path().join(".innen/journal.jsonl");
    std::fs::create_dir_all(&journal_path).expect("create directory at journal path");

    let output = relate(
        kb.path(),
        &[
            "--from",
            "task:one",
            "--edge",
            "BELONGS_TO",
            "--to",
            "project:one",
            "--allow-dangling",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read graph journal"));
}

use std::fs;
use std::io::Write as _;

use serde_json::json;

use crate::journal::Journal;

use super::run;

/// Shared fixture: two nodes plus one valid edge, all via
/// [`Journal::append`] (no hand-written journal bytes).
fn fixture_two_nodes_one_edge(root: &std::path::Path) {
    let journal = Journal::open(root).expect("open");
    journal
        .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
        .expect("append n:1");
    journal
        .append("node.upsert", &json!({"id": "n:2", "label": "two"}))
        .expect("append n:2");
    journal
        .append(
            "edge.assert",
            &json!({"from": "n:1", "to": "n:2", "type": "FOLLOWS_UP"}),
        )
        .expect("append edge");
}

fn journal_bytes(root: &std::path::Path) -> Vec<u8> {
    fs::read(root.join(".innen/journal.jsonl")).expect("read journal bytes")
}

/// Sorted recursive listing of `.innen` (relative paths) for no-mutation pins.
fn snapshot_innen(root: &std::path::Path) -> Vec<String> {
    let base = root.join(".innen");
    let mut out = Vec::new();
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir snapshot") {
            let entry = entry.expect("dir entry snapshot");
            let path = entry.path();
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            out.push(rel);
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn doctor_healthy_is_0() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    let before = journal_bytes(dir.path());

    let report = run(dir.path());

    assert_eq!(report.exit_code, 0, "healthy repo must exit 0: {report:?}");
    let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "journal-valid",
            "quarantine-list",
            "index-rebuildable",
            "ref-integrity",
            "tap_cursor"
        ],
        "checks in spec order: {report:?}"
    );
    for check in &report.checks {
        assert!(check.ok, "{} must be ok: {}", check.name, check.detail);
    }

    // JSON shape pin: {checks:[{name, ok, detail}], exit_code}.
    let value = serde_json::to_value(&report).expect("report serializes");
    assert_eq!(value.get("exit_code").and_then(|v| v.as_u64()), Some(0));
    let checks = value
        .get("checks")
        .and_then(|v| v.as_array())
        .expect("checks is an array");
    assert_eq!(checks.len(), 5);
    assert_eq!(
        checks[0].get("name").and_then(|v| v.as_str()),
        Some("journal-valid")
    );

    // Report-only: journal bytes untouched, no derived index built.
    assert_eq!(journal_bytes(dir.path()), before);
    assert!(!dir.path().join(".innen/index.redb").exists());
    assert!(!dir.path().join(".innen/fts").exists());
}

#[test]
fn a_cursor_that_cannot_name_its_inputs_is_reported_not_silent() {
    // A pre-v2 count cursor left the inbox backlog unknowable: the tap
    // either skipped real files or reported nothing to do. Both states
    // must surface as a failing check with the repair named.
    for (contents, expect) in [("875\n", "count cursor"), ("{not json", "unreadable")] {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        let path = crate::tap::watermark_path(dir.path(), "harvest-dir");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents.as_bytes()).unwrap();

        let report = run(dir.path());
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "tap_cursor")
            .expect("tap_cursor check present");
        assert!(!check.ok, "{contents:?} must fail the check: {check:?}");
        assert!(
            check.detail.contains(expect),
            "detail must name the fault: {}",
            check.detail
        );
        assert_eq!(report.exit_code, 1, "{contents:?} degrades to exit 1");
    }
}

#[test]
fn doctor_quarantine_is_1() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    // Preseed one corrupt line *in the quarantine dir*. The journal
    // itself stays valid: journal corruption is exit 2, while quarantine
    // presence alone is exit 1.
    let qdir = dir.path().join(".innen/quarantine");
    fs::create_dir_all(&qdir).expect("mkdir quarantine");
    fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed quarantine");
    let before = journal_bytes(dir.path());

    let report = run(dir.path());

    assert_eq!(
        report.exit_code, 1,
        "quarantine present must exit 1: {report:?}"
    );
    assert_eq!(report.checks[1].name, "quarantine-list");
    assert!(
        !report.checks[1].ok,
        "quarantine-list must be not-ok: {}",
        report.checks[1].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok: {}",
        report.checks[0].detail
    );

    // Report-only: journal and preseeded quarantine both untouched.
    assert_eq!(journal_bytes(dir.path()), before);
    assert_eq!(
        fs::read_to_string(qdir.join("2026-01-01.jsonl")).expect("reread quarantine"),
        "not json at all\n"
    );
}

#[test]
fn doctor_stale_index_is_1() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    crate::index::build(dir.path()).expect("build");
    // Post-build append: the journal moves on, the derived index falls
    // behind (3 indexed events vs 4 journal events).
    let journal = Journal::open(dir.path()).expect("reopen");
    journal
        .append("node.upsert", &json!({"id": "n:3", "label": "three"}))
        .expect("append n:3");
    drop(journal);
    let before = journal_bytes(dir.path());
    // No-mutation pins with a present index: mtimes + full listing.
    let redb_file = dir.path().join(".innen/index.redb");
    let fts_dir = dir.path().join(".innen/fts");
    let redb_mtime = fs::metadata(&redb_file)
        .expect("redb meta")
        .modified()
        .expect("redb mtime");
    let fts_mtime = fs::metadata(&fts_dir)
        .expect("fts meta")
        .modified()
        .expect("fts mtime");
    let before_listing = snapshot_innen(dir.path());

    let report = run(dir.path());

    assert_eq!(report.exit_code, 1, "stale index must exit 1: {report:?}");
    assert_eq!(report.checks[2].name, "index-rebuildable");
    assert!(
        !report.checks[2].ok,
        "index-rebuildable must be not-ok: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2]
            .detail
            .contains("index has 3 events, journal has 4"),
        "detail names both counts: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok: {}",
        report.checks[0].detail
    );
    assert!(
        report.checks[1].ok,
        "quarantine-list stays ok: {}",
        report.checks[1].detail
    );
    assert!(
        report.checks[3].ok,
        "ref-integrity stays ok: {}",
        report.checks[3].detail
    );

    // Report-only: the stale report leaves the journal untouched.
    assert_eq!(journal_bytes(dir.path()), before);
    // Report-only with a present index: derived bytes untouched.
    assert_eq!(
        fs::metadata(&redb_file)
            .expect("reread redb meta")
            .modified()
            .expect("reread redb mtime"),
        redb_mtime,
        "index.redb mtime must not change"
    );
    assert_eq!(
        fs::metadata(&fts_dir)
            .expect("reread fts meta")
            .modified()
            .expect("reread fts mtime"),
        fts_mtime,
        "fts/ mtime must not change"
    );
    assert_eq!(
        snapshot_innen(dir.path()),
        before_listing,
        "no files added/removed under .innen"
    );

    // Rebuild converges: a fresh index matches the journal again.
    crate::index::rebuild(dir.path()).expect("rebuild");
    let report = run(dir.path());
    assert_eq!(report.exit_code, 0, "rebuilt index must exit 0: {report:?}");
    for check in &report.checks {
        assert!(check.ok, "{} must be ok: {}", check.name, check.detail);
    }
}

#[test]
fn doctor_accepts_rebuilt_index_with_duplicate_event_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    let journal = Journal::open(dir.path()).expect("reopen");
    journal
        .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
        .expect("duplicate n:1");
    drop(journal);
    crate::index::build(dir.path()).expect("build");
    let report = run(dir.path());
    assert_eq!(
        report.exit_code, 0,
        "duplicate IDs are represented by latest values: {report:?}"
    );
    assert!(report.checks[2].ok, "rebuilt duplicate index must be fresh");
}

#[test]
fn doctor_detects_same_id_content_change_without_mutating_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    crate::index::build(dir.path()).expect("build");
    let journal = Journal::open(dir.path()).expect("reopen");
    journal
        .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
        .expect("duplicate n:1");
    drop(journal);
    let journal_path = dir.path().join(".innen/journal.jsonl");
    let mut lines: Vec<String> = fs::read_to_string(&journal_path)
        .expect("journal")
        .lines()
        .map(str::to_owned)
        .collect();
    let last = lines.last_mut().expect("duplicate line");
    *last = last.replacen(
        "\"observed_utc\":\"",
        "\"observed_utc\":\"2099-01-01T00:00:00Z\",\"original_observed_utc\":\"",
        1,
    );
    fs::write(&journal_path, format!("{}\n", lines.join("\n"))).expect("rewrite fixture");
    let before_index = snapshot_innen(dir.path());
    let report = run(dir.path());
    assert_eq!(
        report.exit_code, 1,
        "same ID with changed content is stale: {report:?}"
    );
    assert!(report.checks[2].detail.contains("content differs"));
    assert_eq!(
        snapshot_innen(dir.path()),
        before_index,
        "doctor is read-only"
    );
}

#[test]
fn doctor_dangling_edge_is_2() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = Journal::open(dir.path()).expect("open");
    journal
        .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
        .expect("append n:1");
    // Dangling edge: `to` resolves to neither a known node id nor a Uri.
    // Note: unparseable journal lines cannot produce exit 2 through
    // `Journal::open` (it quarantines them into exit 1), so exit 2 is
    // pinned via ref-integrity instead.
    journal
        .append(
            "edge.assert",
            &json!({"from": "n:1", "to": "n:ghost", "type": "FOLLOWS_UP"}),
        )
        .expect("append dangling edge");
    drop(journal);

    let report = run(dir.path());

    assert_eq!(report.exit_code, 2, "dangling edge must exit 2: {report:?}");
    assert_eq!(report.checks[3].name, "ref-integrity");
    assert!(
        !report.checks[3].ok,
        "ref-integrity must be not-ok: {}",
        report.checks[3].detail
    );
    assert!(
        report.checks[3].detail.contains("n:ghost"),
        "detail names the dangling endpoint: {}",
        report.checks[3].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok (dangling edge parses): {}",
        report.checks[0].detail
    );
    assert!(
        report.checks[1].ok,
        "quarantine-list stays ok: {}",
        report.checks[1].detail
    );
    assert!(
        report.checks[2].ok,
        "index-rebuildable stays ok (no derived index, clean journal): {}",
        report.checks[2].detail
    );
}

#[test]
fn exit_2_dominates_1() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = Journal::open(dir.path()).expect("open");
    journal
        .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
        .expect("append n:1");
    // Dangling edge pins exit 2 via ref-integrity (see
    // `doctor_dangling_edge_is_2`).
    journal
        .append(
            "edge.assert",
            &json!({"from": "n:1", "to": "n:ghost", "type": "FOLLOWS_UP"}),
        )
        .expect("append dangling edge");
    drop(journal);
    // Quarantine presence alone pins exit 1; severity 2 must dominate.
    let qdir = dir.path().join(".innen/quarantine");
    fs::create_dir_all(&qdir).expect("mkdir quarantine");
    fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed quarantine");

    let report = run(dir.path());

    assert_eq!(
        report.exit_code, 2,
        "exit 2 must dominate exit 1: {report:?}"
    );
    assert!(
        !report.checks[1].ok,
        "quarantine-list must be not-ok: {}",
        report.checks[1].detail
    );
    assert!(
        !report.checks[3].ok,
        "ref-integrity must be not-ok: {}",
        report.checks[3].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok (dangling edge parses): {}",
        report.checks[0].detail
    );
    assert!(
        report.checks[2].ok,
        "index-rebuildable stays ok (no derived index, clean journal): {}",
        report.checks[2].detail
    );
}

#[test]
fn partial_index_reports() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    crate::index::build(dir.path()).expect("build");
    // Leave a partial derived index: index.redb present, fts/ missing.
    fs::remove_dir_all(dir.path().join(".innen/fts")).expect("remove fts");
    assert!(dir.path().join(".innen/index.redb").is_file());
    assert!(!dir.path().join(".innen/fts").exists());

    let report = run(dir.path());

    assert_eq!(report.exit_code, 1, "partial index must exit 1: {report:?}");
    assert_eq!(report.checks[2].name, "index-rebuildable");
    assert!(
        !report.checks[2].ok,
        "index-rebuildable must be not-ok: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2].detail.contains("partial"),
        "detail says partial: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2].detail.contains("fts/"),
        "detail names the missing side: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2].detail.contains("rebuildable"),
        "clean journal is rebuildable: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok: {}",
        report.checks[0].detail
    );
    assert!(
        report.checks[1].ok,
        "quarantine-list stays ok: {}",
        report.checks[1].detail
    );
    assert!(
        report.checks[3].ok,
        "ref-integrity stays ok: {}",
        report.checks[3].detail
    );
}

#[test]
fn present_unopenable_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    crate::index::build(dir.path()).expect("build");
    // Present-but-unopenable redb alongside a valid fts/ dir.
    fs::write(dir.path().join(".innen/index.redb"), b"garbage-not-redb").expect("corrupt redb");

    let report = run(dir.path());

    assert_eq!(
        report.exit_code, 1,
        "unopenable index must exit 1: {report:?}"
    );
    assert_eq!(report.checks[2].name, "index-rebuildable");
    assert!(
        !report.checks[2].ok,
        "index-rebuildable must be not-ok: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2].detail.contains("index.redb"),
        "detail names the unreadable side: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[2].detail.contains("rebuildable"),
        "clean journal is rebuildable: {}",
        report.checks[2].detail
    );
    assert!(
        !report.checks[2].detail.contains("not rebuildable"),
        "clean journal must not say not-rebuildable: {}",
        report.checks[2].detail
    );
    assert!(
        report.checks[0].ok,
        "journal-valid stays ok: {}",
        report.checks[0].detail
    );
    assert!(
        report.checks[1].ok,
        "quarantine-list stays ok: {}",
        report.checks[1].detail
    );
    assert!(
        report.checks[3].ok,
        "ref-integrity stays ok: {}",
        report.checks[3].detail
    );
}

#[test]
fn doctor_never_mutates() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture_two_nodes_one_edge(dir.path());
    // Sabotage the journal tail directly: run() must report it, not heal it.
    let journal_path = dir.path().join(".innen/journal.jsonl");
    {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .expect("reopen journal");
        f.write_all(b"garbage-not-json\n").expect("sabotage");
    }
    let before = fs::read(&journal_path).expect("journal bytes");
    assert!(before.ends_with(b"garbage-not-json\n"));

    let report = run(dir.path());

    assert_eq!(
        report.exit_code, 2,
        "corrupt journal must exit 2: {report:?}"
    );
    assert!(
        !report.checks[0].ok,
        "journal-valid must be not-ok: {}",
        report.checks[0].detail
    );
    // Nothing healed, nothing built: corrupt bytes still in place, no
    // quarantine move, no derived index.
    assert_eq!(fs::read(&journal_path).expect("reread journal"), before);
    assert!(
        !dir.path().join(".innen/quarantine").exists(),
        "run must not quarantine"
    );
    assert!(!dir.path().join(".innen/index.redb").exists());
    assert!(!dir.path().join(".innen/fts").exists());
}

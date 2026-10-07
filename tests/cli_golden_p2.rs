//! Byte-exact CLI golden cases.
//!
//! 1. guide byte-exact JSON (pinned text),
//! 2. status counts,
//! 3. timeline month filter,
//! 4. project unknown (exit 1, pinned raw stderr),
//! 5. profile fixture byte-exact vs expected file,
//! 6. artifact add roundtrip (bytes identical + pinned sha256),
//! 7. harvest --check JSON shape (dry-run, no `.innen` side effects),
//! 8. ingest golden journal diff (credential skip + watermark advance).
//!
//! JSON cases compare compact `serde_json::to_string` outputs directly.
//! Each case builds its own temp KB. KB root via explicit `--root <dir>`.
//! No network.
//!
//! NOTE (P1/P2 error prefix): P1 arms print `error: {e}` to stderr, but P2
//! arms intentionally print the core message raw (`{e}`, no prefix) so the
//! plan-pinned literal `unknown project: <id>` matches byte-for-byte.
//! Do not "unify" P2 to `error:` — that would break the plan literal.

use assert_cmd::Command;
use serde_json::json;

// Wire structs mirroring `src/main.rs` P2 JSON shapes (field order is the
// wire order). Expected values use `to_string` on these structs so byte-exact
// comparison is order-stable (avoids `json!` map-ordering drift).

#[derive(serde::Serialize)]
struct WantGuide {
    text: String,
}

#[derive(serde::Serialize)]
struct WantStatus {
    nodes: u64,
    edges: u64,
    events: u64,
    artifacts: u64,
}

#[derive(serde::Serialize)]
struct WantTimelineEntry {
    observed_utc: String,
    op: String,
    summary: String,
}

#[derive(serde::Serialize)]
struct WantTimeline {
    entries: Vec<WantTimelineEntry>,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Append one journal event via core (no CLI).
fn append(root: &std::path::Path, op: &str, payload: serde_json::Value) {
    let journal = innen_core::journal::Journal::open(root).expect("journal open for fixture");
    journal.append(op, &payload).expect("append fixture event");
}

/// Rewrite stored envelope `observed_utc` timestamps in order (for timeline
/// fixtures; `Journal::append` stamps wall-clock and ignores payload time).
///
/// NOTE (coupling): this mirrors the ad-hoc rewrite in
/// `crates/innen-core/src/parity.rs` timeline test (`timeline_filters_by_month`).
/// Coupled to the journal envelope shape (`.innen/journal.jsonl`, one compact
/// JSON object per line with an `observed_utc` field). If the envelope gains a
/// backdate API, prefer that over rewriting here.
fn rewrite_observed(root: &std::path::Path, stamps: &[&str]) {
    let jpath = root.join(".innen/journal.jsonl");
    let content = std::fs::read_to_string(&jpath).expect("read journal");
    let mut lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).expect("journal line parses"))
        .collect();
    assert_eq!(lines.len(), stamps.len(), "fixture event count");
    for (line, stamp) in lines.iter_mut().zip(stamps.iter()) {
        line["observed_utc"] = serde_json::json!(stamp);
    }
    let out = lines
        .iter()
        .map(|v| serde_json::to_string(v).expect("line serializes"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&jpath, out).expect("rewrite journal");
}

// ---------------------------------------------------------------------------
// Case 1: guide byte-exact JSON (pinned text)
// ---------------------------------------------------------------------------

#[test]
fn guide_contains_query() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "guide"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    // Pinned guide text — byte-exact means byte-exact. Hardcoded here (not
    // `guide_text()`) so drift fails the test. Keep in sync with
    // `innen_core::parity::guide_text`.
    const GUIDE_TEXT: &str = "innen is a local-first knowledge graph over an append-only journal.\nWhat remains? project [id] returns compact recorded tasks across projects or one project.\nNeed evidence? project <id> --view evidence; legacy project page: --view full.\nFind prior knowledge: query --q <term>. Continue a conversation: resume <session-id>.\nTrace original sources: wiki sync after wiki edits, then trace --q <clue>; use returned expand_argv for exact records.\nTask status is recorded evidence; unknown status and heuristic conversation candidates are not confirmed unfinished work.";
    // Exact first line + contains query (robustness probes on the decoded text).
    let first_line = GUIDE_TEXT.lines().next().expect("guide has first line");
    assert_eq!(
        first_line, "innen is a local-first knowledge graph over an append-only journal.",
        "guide first line must be exact"
    );
    assert!(
        GUIDE_TEXT.contains("query"),
        "guide text must contain query"
    );
    // Full stdout EQUALS pinned expected JSON (single line + trailing newline).
    let want = serde_json::to_string(&WantGuide {
        text: GUIDE_TEXT.to_string(),
    })
    .expect("want serializes");
    assert_eq!(out, format!("{want}\n"), "guide stdout must be byte-exact");
}

// ---------------------------------------------------------------------------
// Case 2: status counts 3-node/1-edge
// ---------------------------------------------------------------------------

#[test]
fn status_counts() {
    let dir = tempfile::tempdir().expect("tempdir");
    for id in ["a:1", "a:2", "a:3"] {
        append(
            dir.path(),
            "node.upsert",
            json!({"id": id, "type": "Task", "label": id}),
        );
    }
    append(
        dir.path(),
        "edge.assert",
        json!({"from": "a:1", "type": "FOLLOWS_UP", "to": "a:2"}),
    );

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "status"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let got = out.trim_end().to_string();

    let want = serde_json::to_string(&WantStatus {
        nodes: 3,
        edges: 1,
        events: 4,
        artifacts: 0,
    })
    .expect("want serializes");
    assert_eq!(got, want, "status counts must be byte-exact");
}

// ---------------------------------------------------------------------------
// Case 3: timeline month filter
// ---------------------------------------------------------------------------

#[test]
fn timeline_month() {
    let dir = tempfile::tempdir().expect("tempdir");
    append(
        dir.path(),
        "node.upsert",
        json!({"id": "a:1", "type": "Task", "label": "aug"}),
    );
    append(
        dir.path(),
        "node.upsert",
        json!({"id": "s:1", "type": "Task", "label": "sep"}),
    );
    rewrite_observed(
        dir.path(),
        &["2026-08-01T00:00:00Z", "2026-09-01T00:00:00Z"],
    );

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "timeline", "2026-09"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let got = out.trim_end().to_string();

    let want = serde_json::to_string(&WantTimeline {
        entries: vec![WantTimelineEntry {
            observed_utc: "2026-09-01T00:00:00Z".to_string(),
            op: "node.upsert".to_string(),
            summary: "sep".to_string(),
        }],
    })
    .expect("want serializes");
    assert_eq!(got, want, "timeline 2026-09 must return September only");
}

// ---------------------------------------------------------------------------
// Case 4: project unknown → exit 1, stderr byte-exact
// ---------------------------------------------------------------------------

#[test]
fn project_unknown() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "project", "p:nope"])
        .assert()
        .code(1);
    let err = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr utf8");
    // Plan Case 5 pins stderr `unknown project: p:nope` raw (no `error: `
    // prefix). P1 arms use `error: {e}`; P2 intentionally keeps raw `{e}`.
    // Do not unify — see file header NOTE.
    assert_eq!(
        err, "unknown project: p:nope\n",
        "stderr must equal plan-pinned literal, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Case 5: profile fixture byte-exact vs expected file
// ---------------------------------------------------------------------------

#[test]
fn profile_fixture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_toml = std::fs::read_to_string(manifest_dir.join("tests/fixtures/profile.toml"))
        .expect("read profile fixture");
    let expected = std::fs::read_to_string(manifest_dir.join("tests/fixtures/expected_profile.md"))
        .expect("read expected");
    std::fs::create_dir_all(dir.path().join("profile")).expect("mkdir profile");
    std::fs::write(dir.path().join("profile/profile.toml"), &fixture_toml)
        .expect("write profile.toml");

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "human", "profile"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    assert_eq!(
        out, expected,
        "profile render must be byte-exact vs expected file"
    );
}

// ---------------------------------------------------------------------------
// Case 6: artifact add roundtrip (stored bytes identical)
// ---------------------------------------------------------------------------

#[test]
fn artifact_roundtrip_cli() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("note.txt");
    std::fs::write(&src, b"hello-bytes").expect("write src");
    let root = dir.path().to_string_lossy().into_owned();
    let src_s = src.to_string_lossy().into_owned();

    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root", &root, "--format", "json", "artifact", "add", "--file", &src_s,
        ])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let v: serde_json::Value =
        serde_json::from_str(out.trim_end()).expect("artifact stdout parses as JSON");
    // sha256 of the 11 bytes `hello-bytes`, verified against `sha256sum`.
    const WANT_SHA256: &str = "dd1fb82ed53df87c98fa9397b0ceba6b166374eab810ab351e5d955652613f37";
    assert_eq!(
        v.get("sha256").and_then(|x| x.as_str()),
        Some(WANT_SHA256),
        "artifact sha256 must equal independently computed hash"
    );
    assert_eq!(
        v.get("bytes").and_then(serde_json::Value::as_u64),
        Some(11),
        "artifact bytes must be 11"
    );
    let stored = v
        .get("path")
        .and_then(|x| x.as_str())
        .expect("artifact JSON has path");
    let bytes = std::fs::read(stored).expect("read stored artifact");
    assert_eq!(bytes, b"hello-bytes", "stored bytes must be identical");
}

#[test]
fn artifact_tree_cli_archives_metadata_not_source_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source-tree");
    std::fs::create_dir(&source).expect("mkdir source");
    std::fs::write(source.join("data.bin"), b"private-source-bytes").expect("write source");
    let manifest = dir.path().join("receipts/tree.json");
    let root = dir.path().join("kb");
    let journal = innen_core::journal::Journal::open(&root).expect("journal open");
    journal
        .append(
            "node.upsert",
            &serde_json::json!({"id":"project:test","type":"Project","label":"Test Project"}),
        )
        .expect("seed project");
    drop(journal);
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root",
            root.to_str().unwrap(),
            "artifact",
            "add-tree",
            "--directory",
            source.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--project",
            "project:test",
        ])
        .assert()
        .code(0);
    let value: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("JSON output");
    assert_eq!(value["files"], 1);
    assert_eq!(value["source_bytes"], 20);
    assert_eq!(value["metadata_only"], true);
    assert!(manifest.is_file());
    assert_eq!(
        std::fs::read(source.join("data.bin")).unwrap(),
        b"private-source-bytes"
    );
}

// ---------------------------------------------------------------------------
// Cases 7-8: harvest --check + ingest over Tap.
// Wire structs mirror the core `harvest` JSON shapes (field order is the
// wire order); expected values use `to_string` so comparison is byte-exact.
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct WantHarvestTap {
    id: String,
    new_files: Vec<String>,
    skipped: Vec<String>,
    cursor: String,
}

#[derive(serde::Serialize)]
struct WantHarvest {
    taps: Vec<WantHarvestTap>,
}

#[derive(serde::Serialize)]
struct WantIngestSkipped {
    path: String,
    pattern: String,
    preview: String,
}

#[derive(serde::Serialize)]
struct WantIngest {
    added: u64,
    skipped: Vec<WantIngestSkipped>,
}

// ---------------------------------------------------------------------------
// Case 7: harvest --check JSON shape (dry-run, no side effects)
// ---------------------------------------------------------------------------

#[test]
fn harvest_check_json_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inbox = dir.path().join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).expect("mkdir inbox");
    std::fs::write(inbox.join("a.md"), "hello").expect("write a.md");
    std::fs::write(inbox.join("b.md"), "world").expect("write b.md");

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "harvest", "--check"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let got = out.trim_end().to_string();

    let want = serde_json::to_string(&WantHarvest {
        taps: vec![WantHarvestTap {
            id: "harvest-dir".to_string(),
            new_files: vec!["a.md".to_string(), "b.md".to_string()],
            skipped: vec![],
            cursor: "missing".to_string(),
        }],
    })
    .expect("want serializes");
    assert_eq!(got, want, "harvest --check must list new files byte-exact");
    // Dry-run pins: no journal appends, no watermark advance (`.innen`
    // untouched — `check` never opens the journal nor stores a watermark).
    assert!(
        !dir.path().join(".innen").exists(),
        "harvest --check must not create .innen"
    );
}

// ---------------------------------------------------------------------------
// Case 8: ingest golden journal diff (credential skip + watermark)
// ---------------------------------------------------------------------------

#[test]
fn ingest_journal_diff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inbox = dir.path().join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).expect("mkdir inbox");
    std::fs::write(inbox.join("ok.md"), "hello").expect("write ok.md");
    std::fs::write(inbox.join("bad.md"), "key AKIAIOSFODNN7EXAMPLE end").expect("write bad.md");

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "ingest"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let got = out.trim_end().to_string();

    let want = serde_json::to_string(&WantIngest {
        added: 1,
        skipped: vec![WantIngestSkipped {
            path: "bad.md".to_string(),
            pattern: "aws-access-key".to_string(),
            preview: "AKIAIO***".to_string(),
        }],
    })
    .expect("want serializes");
    assert_eq!(got, want, "ingest report must be byte-exact");

    // Journal diff: exactly one `node.upsert` (ok.md); bad.md never appended.
    let journal =
        std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).expect("read journal");
    assert_eq!(
        journal.lines().count(),
        1,
        "ingest must append exactly one event, got: {journal:?}"
    );
    assert!(journal.contains("ok.md"), "journal must record ok.md");
    assert!(
        !journal.contains("bad.md"),
        "credential file must never be appended"
    );
    // The cursor names every consumed file, added and skipped alike. A count
    // would break here: the pending files are deleted after ingest, which is
    // what silently swallowed the whole inbox for 17 days.
    let cursor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".innen/tap/harvest-dir.watermark"))
            .expect("read cursor"),
    )
    .expect("cursor is JSON");
    assert_eq!(cursor["schema"], "innen.tap.watermark.v2");
    let consumed: Vec<&str> = cursor["consumed"]
        .as_array()
        .expect("consumed is an array")
        .iter()
        .map(|v| v.as_str().expect("name is a string"))
        .collect();
    assert_eq!(consumed, vec!["bad.md", "ok.md"], "both files are consumed");
}

#[test]
fn harvest_check_reports_a_rebuilt_cursor_instead_of_an_empty_backlog() {
    // The stuck state: a pre-v2 count cursor with a full inbox. Reporting
    // "nothing new" here is how 96 files sat unprocessed without an error.
    let dir = tempfile::tempdir().expect("tempdir");
    let inbox = dir.path().join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).expect("mkdir inbox");
    std::fs::write(inbox.join("a.md"), "hello").expect("write a.md");
    let cursor = dir.path().join(".innen/tap/harvest-dir.watermark");
    std::fs::create_dir_all(cursor.parent().unwrap()).expect("mkdir tap");
    std::fs::write(&cursor, "875").expect("write legacy count");

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "harvest", "--check"])
        .assert()
        .code(0);
    let report: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout parses");
    assert_eq!(report["taps"][0]["cursor"], "legacy_count_rebuilt");
    assert_eq!(
        report["taps"][0]["new_files"],
        serde_json::json!(["a.md"]),
        "the inbox must be re-listed, not reported empty"
    );
}

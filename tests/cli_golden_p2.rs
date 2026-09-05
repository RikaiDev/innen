//! CLI golden P2 (Task 12).
//!
//! Eight cases, thin dispatch over core fns from Tasks 9-11:
//! 1. guide contains "query",
//! 2. search lexical fixture,
//! 3. status counts,
//! 4. timeline month filter,
//! 5. project unknown (exit 1),
//! 6. profile fixture byte-exact vs expected file,
//! 7. artifact add roundtrip (bytes identical),
//! 8. cloud status stub via PATH.
//!
//! JSON cases compare compact `serde_json::to_string` outputs directly
//! (byte-exact, like Task 7's strengthened assertion). Each case builds its
//! own temp KB. KB root via explicit `--root <dir>`. No network.

use assert_cmd::Command;
use serde_json::json;

// Wire structs mirroring `src/main.rs` P2 JSON shapes (field order is the
// wire order). Expected values use `to_string` on these structs so byte-exact
// comparison is order-stable (avoids `json!` map-ordering drift).

#[derive(serde::Serialize)]
struct WantSearchHit {
    node_id: String,
    excerpt: String,
}

#[derive(serde::Serialize)]
struct WantSearch {
    hits: Vec<WantSearchHit>,
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
// Case 1: guide contains "query"
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
    assert!(
        out.contains("query"),
        "guide stdout must contain query, got: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// Case 2: search lexical fixture → exactly [a:1]
// ---------------------------------------------------------------------------

#[test]
fn search_lexical_fixture() {
    let dir = tempfile::tempdir().expect("tempdir");
    append(
        dir.path(),
        "node.upsert",
        json!({"id": "a:1", "type": "Task", "label": "SFT tokenizer"}),
    );
    append(
        dir.path(),
        "node.upsert",
        json!({"id": "b:2", "type": "Task", "label": "unrelated chores"}),
    );
    append(
        dir.path(),
        "edge.assert",
        json!({"from": "b:2", "type": "FOLLOWS_UP", "to": "a:1"}),
    );

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root",
            &root,
            "--format",
            "json",
            "search",
            "--keyword",
            "SFT",
        ])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let got = out.trim_end().to_string();

    // Byte-exact: compact to_string on both sides, field order
    // hits -> node_id, excerpt (struct-to-struct, not json! map order).
    let want = serde_json::to_string(&WantSearch {
        hits: vec![WantSearchHit {
            node_id: "a:1".to_string(),
            excerpt: "SFT tokenizer".to_string(),
        }],
    })
    .expect("want serializes");
    assert_eq!(got, want, "search SFT must hit exactly [a:1]");
}

// ---------------------------------------------------------------------------
// Case 3: status counts 3-node/1-edge
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
// Case 4: timeline month filter
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
// Case 5: project unknown → exit 1, stderr
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
    assert!(
        err.contains("unknown project: p:nope"),
        "stderr must contain unknown project, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Case 6: profile fixture byte-exact vs expected file
// ---------------------------------------------------------------------------

#[test]
fn profile_fixture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixture_toml =
        std::fs::read_to_string("tests/fixtures/profile.toml").expect("read profile fixture");
    let expected =
        std::fs::read_to_string("tests/fixtures/expected_profile.md").expect("read expected");
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
// Case 7: artifact add roundtrip (stored bytes identical)
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
    let stored = v
        .get("path")
        .and_then(|x| x.as_str())
        .expect("artifact JSON has path");
    let bytes = std::fs::read(stored).expect("read stored artifact");
    assert_eq!(bytes, b"hello-bytes", "stored bytes must be identical");
}

// ---------------------------------------------------------------------------
// Case 8: cloud status stub via PATH
// ---------------------------------------------------------------------------

#[test]
fn cloud_status_stub() {
    // CLI resolves `rclone` via PATH lookup. Link the canned stub
    // (crates/innen-core/tests/fixtures/fake-rclone.sh) as `rclone` in a
    // temp bin dir and prepend it to PATH.
    let dir = tempfile::tempdir().expect("tempdir");
    let bindir = tempfile::tempdir().expect("bindir");
    let stub = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("crates/innen-core/tests/fixtures/fake-rclone.sh");
    assert!(stub.is_file(), "stub must exist: {}", stub.display());
    let link = bindir.path().join("rclone");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&stub, &link).expect("symlink rclone");
    #[cfg(not(unix))]
    std::fs::copy(&stub, &link).expect("copy rclone");

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bindir.path().display(), old_path);

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .env("PATH", &new_path)
        .args(["--root", &root, "--format", "json", "cloud", "status"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    assert!(
        out.contains("canned-dir"),
        "cloud status must print canned listing, got: {out:?}"
    );
}

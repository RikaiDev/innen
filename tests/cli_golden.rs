//! CLI golden + QC contract (Task 8d, P1).
//!
//! Six cases, all `--format json` byte-exact (compact `serde_json::to_string`
//! on structs, single line + trailing newline; tests compare `trim_end()`):
//! 1. query golden via CLI (core `golden-query.json` "normal" case),
//! 2. empty-KB query (exit 0 + empty hits),
//! 3. doctor healthy (exit 0),
//! 4. doctor quarantine (exit 1),
//! 5. config set precedence (env override),
//! 6. completions bash non-empty (loops all four shells).
//!
//! KB root is always passed as explicit `--root <dir>` (precedence
//! `--root > INNEN_ROOT > cwd` lives in the binary; tests never rely on cwd).
//! No network. Per-command env via `Command::env` (no process-env races).

use assert_cmd::Command;
use serde_json::Value;

/// Core golden fixture (same file the `innen-core` golden test uses).
const CORE_GOLDEN: &str = include_str!("../crates/innen-core/tests/golden-query.json");

/// Build a temp KB from a core golden case's `journal` array via
/// `innen_core::journal::Journal::append` (no CLI, no network).
fn build_kb_from_case(dir: &tempfile::TempDir, case: &Value) {
    let journal = innen_core::journal::Journal::open(dir.path()).expect("journal open for fixture");
    for ev in case["journal"].as_array().expect("journal array") {
        journal
            .append(
                ev["op"].as_str().expect("op str"),
                ev.get("payload").expect("payload"),
            )
            .expect("append fixture event");
    }
}

/// Find a named case in the core golden file.
fn golden_case(name: &str) -> Value {
    let cases: Vec<Value> = serde_json::from_str(CORE_GOLDEN).expect("core fixture parses");
    cases
        .into_iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("core fixture must contain case {name:?}"))
}

/// Run the CLI with explicit `--root` + `--format json` and return
/// (exit code, trimmed stdout). Asserts success/failure via `code()` at
/// call sites; this helper only captures output for byte comparison.
fn cli_stdout_trimmed(args: &[&str]) -> (i32, String) {
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(args)
        .assert();
    let code = assert
        .get_output()
        .status
        .code()
        .expect("process exited with code");
    let out = assert.get_output().stdout.clone();
    let s = String::from_utf8(out).expect("stdout is utf8");
    (code, s.trim_end().to_string())
}

#[test]
fn cli_query_golden_via_cli() {
    // Reuse core fixture "normal": 2 nodes + 1 edge, q=SFT, 1 hit, no warnings.
    let case = golden_case("normal");
    let dir = tempfile::tempdir().expect("tempdir");
    build_kb_from_case(&dir, &case);

    let root = dir.path().to_string_lossy().into_owned();
    let q = case["query"]["q"].as_str().expect("q str").to_string();
    let as_of = case["query"]["as_of"]
        .as_str()
        .expect("as_of str")
        .to_string();
    let (code, got) = cli_stdout_trimmed(&[
        "--root", &root, "--format", "json", "query", "--q", &q, "--as-of", &as_of,
    ]);
    assert_eq!(code, 0, "query golden must exit 0, got: {got:?}");

    // Byte-exact: same struct type on both sides, compact `to_string`.
    let want_struct: innen_core::query::QueryOutput =
        serde_json::from_value(case["expected"].clone()).expect("expected parses");
    let want = serde_json::to_string(&want_struct).expect("expected serializes");
    assert_eq!(got, want, "query output must be byte-exact");
}

#[test]
fn cli_empty_kb_query_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let (code, got) = cli_stdout_trimmed(&[
        "--root",
        &root,
        "--format",
        "json",
        "query",
        "--q",
        "qqqzzzqqq",
        "--as-of",
        "2026-09-03T00:00:00Z",
    ]);
    assert_eq!(code, 0, "empty-KB query must exit 0, got: {got:?}");
    let want = serde_json::to_string(&innen_core::query::QueryOutput {
        hits: vec![],
        warnings: vec![],
    })
    .expect("empty output serializes");
    assert_eq!(got, want, "empty-KB query must be byte-exact empty hits");
}

#[test]
fn cli_doctor_healthy_exits_0() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = innen_core::journal::Journal::open(dir.path()).expect("journal open for doctor");
    journal
        .append(
            "node.upsert",
            &serde_json::json!({"id": "n:1", "label": "one"}),
        )
        .expect("append n:1");
    journal
        .append(
            "node.upsert",
            &serde_json::json!({"id": "n:2", "label": "two"}),
        )
        .expect("append n:2");
    journal
        .append(
            "edge.assert",
            &serde_json::json!({"from": "n:1", "to": "n:2", "type": "FOLLOWS_UP"}),
        )
        .expect("append edge");
    drop(journal);

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "doctor"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: innen_core::doctor::Report =
        serde_json::from_str(out.trim_end()).expect("doctor stdout parses as Report");
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.checks.len(), 4);
    for c in &report.checks {
        assert!(c.ok, "{} must be ok: {}", c.name, c.detail);
    }
    // Byte-exact pin: trimmed stdout equals compact struct serialization.
    let want = serde_json::to_string(&report).expect("report serializes");
    assert_eq!(out.trim_end(), want);
}

#[test]
fn cli_doctor_quarantine_exits_1() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = innen_core::journal::Journal::open(dir.path()).expect("journal open for doctor");
    journal
        .append(
            "node.upsert",
            &serde_json::json!({"id": "n:1", "label": "one"}),
        )
        .expect("append n:1");
    journal
        .append(
            "node.upsert",
            &serde_json::json!({"id": "n:2", "label": "two"}),
        )
        .expect("append n:2");
    journal
        .append(
            "edge.assert",
            &serde_json::json!({"from": "n:1", "to": "n:2", "type": "FOLLOWS_UP"}),
        )
        .expect("append edge");
    drop(journal);
    let qdir = dir.path().join(".innen/quarantine");
    std::fs::create_dir_all(&qdir).expect("mkdir quarantine");
    std::fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed");

    let root = dir.path().to_string_lossy().into_owned();
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(["--root", &root, "--format", "json", "doctor"])
        .assert()
        .code(1);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let report: innen_core::doctor::Report =
        serde_json::from_str(out.trim_end()).expect("doctor stdout parses as Report");
    assert_eq!(report.exit_code, 1);
    assert_eq!(report.checks[1].name, "quarantine-list");
    assert!(!report.checks[1].ok);
}

#[test]
fn cli_config_set_precedence_env_overrides() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();

    // `config set format human` → exit 0.
    Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root", &root, "--format", "json", "config", "set", "format", "human",
        ])
        .assert()
        .code(0);

    // Without env, `config get format` returns the file value (human).
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root", &root, "--format", "json", "config", "get", "format",
        ])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let v: Value = serde_json::from_str(out.trim_end()).expect("get output parses");
    assert_eq!(
        v.get("value").and_then(|x| x.as_str()),
        Some("human"),
        "file value must be human, got: {out:?}"
    );

    // With `INNEN_FORMAT=json`, env wins over the file (precedence pin).
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .env("INNEN_FORMAT", "json")
        .args([
            "--root", &root, "--format", "json", "config", "get", "format",
        ])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let v: Value = serde_json::from_str(out.trim_end()).expect("get output parses");
    assert_eq!(
        v.get("value").and_then(|x| x.as_str()),
        Some("json"),
        "env must override file, got: {out:?}"
    );
}

#[test]
fn cli_completions_bash_nonempty() {
    // Per-shell non-empty pin (bash/zsh/fish/powershell) in one case.
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let assert = Command::cargo_bin("innen")
            .expect("cargo bin innen")
            .args(["completions", shell])
            .assert()
            .code(0);
        let out = assert.get_output().stdout.clone();
        assert!(!out.is_empty(), "completions {shell} must be non-empty");
    }
}

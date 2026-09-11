//! `innen hook` CLI contract: pending/run/install, fail-open runtime.
use assert_cmd::Command;

fn cli(args: &[&str]) -> (i32, String) {
    let assert = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args(args)
        .assert();
    let code = assert.get_output().status.code().unwrap_or(-1);
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    (code, out.trim_end().to_string())
}

#[test]
fn hook_pending_empty_kb_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let (code, got) = cli(&["--root", &root, "--format", "json", "hook", "pending"]);
    assert_eq!(code, 0);
    assert_eq!(got, "{\"pending\":[]}");
}

#[test]
fn hook_run_writes_snapshot_then_dedups() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let (code, got) = cli(&[
        "--root",
        &root,
        "--format",
        "json",
        "hook",
        "run",
        "--event",
        "session-end",
        "--cwd",
        dir.path().to_str().expect("cwd"),
    ]);
    assert_eq!(code, 0, "hook run must be fail-open, got: {got:?}");
    let v: serde_json::Value = serde_json::from_str(&got).expect("receipt json");
    let name = v["wrote"].as_str().expect("wrote file");
    assert!(
        name.starts_with("pending-") && name.ends_with(".md"),
        "name: {name:?}"
    );
    let (code2, got2) = cli(&[
        "--root",
        &root,
        "--format",
        "json",
        "hook",
        "run",
        "--event",
        "session-end",
        "--cwd",
        dir.path().to_str().expect("cwd"),
    ]);
    assert_eq!(code2, 0);
    assert!(got2.contains("no-change"), "second run dedups: {got2:?}");
}

#[test]
fn hook_install_claude_code_project_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let cwd = dir.path().to_str().expect("cwd").to_string();
    let args: &[&str] = &[
        "--root",
        &root,
        "--format",
        "json",
        "hook",
        "install",
        "--agent",
        "claude-code",
        "--scope",
        "project",
        "--cwd",
        &cwd,
        "--kb-root",
        &root,
    ];
    let (code, _) = cli(args);
    assert_eq!(code, 0);
    let (code2, _) = cli(args);
    assert_eq!(code2, 0);
    let settings = std::fs::read_to_string(dir.path().join(".claude/settings.json"))
        .expect("settings written");
    let v: serde_json::Value = serde_json::from_str(&settings).expect("settings json");
    let end = v["hooks"]["SessionEnd"]
        .as_array()
        .expect("SessionEnd array");
    assert_eq!(end.len(), 1, "idempotent single entry");
    assert!(end[0]["hooks"][0]["command"]
        .as_str()
        .expect("cmd")
        .contains("session-end"));
    let start = v["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart array");
    assert_eq!(
        start[0]["matcher"],
        serde_json::Value::String("startup|resume".to_string())
    );
}

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
fn hook_run_compact_emits_decision_allow() {
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
        "compact",
        "--cwd",
        dir.path().to_str().expect("cwd"),
    ]);
    assert_eq!(code, 0);
    assert_eq!(got, "{\"decision\":\"allow\"}");
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
    let precompact = v["hooks"]["PreCompact"]
        .as_array()
        .expect("PreCompact array");
    assert_eq!(precompact.len(), 1, "idempotent single PreCompact entry");
    assert!(precompact[0]["hooks"][0]["command"]
        .as_str()
        .expect("cmd")
        .contains("--event compact"));
}

#[test]
fn hook_install_codex_project_adds_stop_and_is_idempotent() {
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
        "codex",
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

    let config =
        std::fs::read_to_string(dir.path().join(".codex/hooks.json")).expect("Codex hooks written");
    let v: serde_json::Value = serde_json::from_str(&config).expect("hooks json");
    let stop = v["hooks"]["Stop"].as_array().expect("Stop array");
    assert_eq!(stop.len(), 1, "idempotent single Stop entry");
    assert!(stop[0]["hooks"][0]["command"]
        .as_str()
        .expect("Stop command")
        .contains("--event stop"));

    let end = v["hooks"]["SessionEnd"]
        .as_array()
        .expect("SessionEnd array");
    assert_eq!(end.len(), 1, "idempotent single SessionEnd entry");
    assert!(end[0]["hooks"][0]["command"]
        .as_str()
        .expect("SessionEnd command")
        .contains("--event session-end"));
}

#[test]
fn hook_install_codex_replaces_bare_innen_handlers_with_absolute_executable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let cwd = dir.path().to_str().expect("cwd").to_string();
    let hooks_dir = dir.path().join(".codex");
    std::fs::create_dir(&hooks_dir).expect("hooks dir");
    std::fs::write(
        hooks_dir.join("hooks.json"),
        r#"{
          "hooks": {
            "Stop": [{"hooks": [{"type": "command", "command": "innen hook run --event stop --kb-root /old"}]}],
            "SessionEnd": [{"hooks": [{"type": "command", "command": "innen hook run --event session-end --kb-root /old"}]}]
          }
        }"#,
    )
    .expect("old hooks");

    let args: &[&str] = &[
        "--root",
        &root,
        "--format",
        "json",
        "hook",
        "install",
        "--agent",
        "codex",
        "--scope",
        "project",
        "--cwd",
        &cwd,
        "--kb-root",
        &root,
    ];
    let (code, _) = cli(args);
    assert_eq!(code, 0);

    let config =
        std::fs::read_to_string(hooks_dir.join("hooks.json")).expect("Codex hooks written");
    let v: serde_json::Value = serde_json::from_str(&config).expect("hooks json");
    for event in ["Stop", "SessionEnd"] {
        let command = v["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .expect("installed command");
        assert!(
            command.starts_with('/'),
            "command must be absolute: {command}"
        );
        assert!(
            !command.starts_with("innen "),
            "stale bare command: {command}"
        );
    }
}

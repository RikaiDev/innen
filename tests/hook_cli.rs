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

/// A worktree with one commit. `hook run` only snapshots Git worktrees, so a
/// snapshot fixture has to be a real repository.
fn init_repo(dir: &std::path::Path) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    run(&["init", "-q"]);
    run(&[
        "-c",
        "user.email=innen@example.invalid",
        "-c",
        "user.name=innen test",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "init",
    ]);
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
    init_repo(dir.path());
    // The KB root must be outside the worktree: an inbox inside the repository
    // would dirty it and change the worktree state that the receipt keys on.
    let kb = tempfile::tempdir().expect("kb tempdir");
    let root = kb.path().to_string_lossy().into_owned();
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

    // A changed worktree is a new receipt, not a suppressed one.
    std::fs::write(dir.path().join("notes.md"), "state changed\n").expect("write");
    let (code3, got3) = cli(&[
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
    assert_eq!(code3, 0);
    let v3: serde_json::Value = serde_json::from_str(&got3).expect("receipt json");
    let name3 = v3["wrote"]
        .as_str()
        .expect("changed state writes a receipt");
    assert_ne!(name3, name, "a changed worktree must not dedup");
}

#[test]
fn hook_run_outside_repository_writes_nothing() {
    // A directory that is not a worktree has no repository, no branch and no
    // diff, so there is nothing to record. It must not be written as
    // `repo: <dir>` with `branch: nongit`.
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
    assert_eq!(v["skipped"], "not-a-repository", "receipt: {got}");
    assert!(v.get("wrote").is_none(), "must not write: {got:?}");
    let (_, pending) = cli(&["--root", &root, "--format", "json", "hook", "pending"]);
    assert_eq!(pending, "{\"pending\":[]}", "inbox must stay empty");
}

#[test]
fn hook_run_records_repository_root_from_subdirectory() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    let sub = dir.path().join("nested/deep");
    std::fs::create_dir_all(&sub).expect("mkdir");
    let kb = dir.path().join("kb");
    let (code, got) = cli(&[
        "--root",
        kb.to_str().expect("kb"),
        "--format",
        "json",
        "hook",
        "run",
        "--event",
        "session-end",
        "--cwd",
        sub.to_str().expect("sub"),
    ]);
    assert_eq!(code, 0, "got: {got:?}");
    let v: serde_json::Value = serde_json::from_str(&got).expect("receipt json");
    let name = v["wrote"].as_str().expect("wrote file");
    let body = std::fs::read_to_string(kb.join("00-inbox/harvest").join(name)).expect("snapshot");
    // git reports the physical path, so compare canonical forms.
    let top = dir
        .path()
        .canonicalize()
        .expect("canonicalize")
        .to_string_lossy()
        .into_owned();
    assert!(
        body.contains(&format!("- repo: {top}\n")),
        "expected repo root {top}, snapshot was:\n{body}"
    );
    assert!(
        !body.contains("branch: nongit"),
        "a worktree must not report an unborn branch:\n{body}"
    );
}

#[test]
fn hook_run_compact_emits_decision_allow() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
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

/// Antigravity's documented hook payload. Verified against the contract
/// embedded in `agy` 1.2.16: camelCase protojson, `workspacePaths` as an array,
/// and no `cwd` key at all — it runs the hook with its cwd set to the
/// directory holding `hooks.json`, which is not a repository.
#[test]
fn hook_run_reads_antigravity_workspace_and_conversation() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    let outside = tempfile::tempdir().expect("outside dir");
    let kb = dir.path().join("kb");
    let payload = serde_json::json!({
        "conversationId": "ec33ebf9-0cba-4100-8142-c61503f6c587",
        "workspacePaths": [dir.path().to_str().expect("ws")],
        "transcriptPath": "/tmp/transcript.jsonl",
        "modelName": "auto",
    })
    .to_string();

    // A cwd that is NOT a repository, to prove the payload wins over it.
    // `session-end` rather than `stop` so the receipt reaches stdout: decision
    // events always print `{"decision":"allow"}` and hide the receipt by design.
    let child = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .current_dir(outside.path())
        .args([
            "--root",
            kb.to_str().expect("kb"),
            "--format",
            "json",
            "hook",
            "run",
            "--event",
            "session-end",
            "--kb-root",
            kb.to_str().expect("kb"),
        ])
        .write_stdin(payload)
        .assert();
    let out = String::from_utf8(child.get_output().stdout.clone()).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(out.trim_end()).expect("receipt json");
    let name = v["wrote"]
        .as_str()
        .unwrap_or_else(|| panic!("Antigravity payload must resolve a workspace: {out}"));
    let body = std::fs::read_to_string(kb.join("00-inbox/harvest").join(name)).expect("snapshot");

    let top = dir
        .path()
        .canonicalize()
        .expect("canonicalize")
        .to_string_lossy()
        .into_owned();
    assert!(
        body.contains(&format!("- repo: {top}\n")),
        "workspacePaths[0] must win over the non-repo cwd:\n{body}"
    );
    assert!(
        body.contains("- session: ec33ebf9-0cba-4100-8142-c61503f6c587\n"),
        "conversationId must become the session identity:\n{body}"
    );
    assert!(
        body.contains("- transcript: /tmp/transcript.jsonl\n"),
        "transcriptPath must be recorded:\n{body}"
    );
}

#[test]
fn hook_run_cwd_flag_overrides_hook_stdin() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    let kb = tempfile::tempdir().expect("kb");
    let payload = serde_json::json!({
        "conversationId": "c-from-stdin",
        "workspacePaths": ["/nonexistent/workspace"],
    })
    .to_string();
    let child = Command::cargo_bin("innen")
        .expect("cargo bin innen")
        .args([
            "--root",
            kb.path().to_str().expect("kb"),
            "--format",
            "json",
            "hook",
            "run",
            "--event",
            "session-end",
            "--kb-root",
            kb.path().to_str().expect("kb"),
            "--cwd",
            dir.path().to_str().expect("cwd"),
        ])
        .write_stdin(payload)
        .assert();
    let out = String::from_utf8(child.get_output().stdout.clone()).expect("utf8");
    assert!(
        out.contains("\"wrote\""),
        "--cwd must override a bad workspacePaths: {out}"
    );
}

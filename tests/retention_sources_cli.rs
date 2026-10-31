//! The retention command line must accept every source the engine can reclaim.
//!
//! Copilot and Grok gained delete adapters, but the command line still
//! rejected their names, so those sources could only be reclaimed through a
//! full-source sweep that no operator would dare run.

use assert_cmd::Command;

/// Every source name the engine can reclaim, aliases included.
const RECLAIMABLE: [&str; 10] = [
    "antigravity",
    "agy",
    "claude",
    "codex",
    "copilot",
    "grok",
    "mcode",
    "minimax",
    "opencode",
    "qwen",
];

/// Run `innen retention plan --source <source>` against an empty knowledge
/// base and an empty home, so the command only exercises argument parsing.
fn plan_source(source: &str) -> std::process::Output {
    let temp = tempfile::tempdir().expect("temp root");
    let mut command = Command::cargo_bin("innen").expect("innen binary");
    command.env("HOME", temp.path());
    command.args([
        "--root",
        temp.path().to_str().expect("utf-8 root"),
        "retention",
        "plan",
        "--source",
        source,
        "--format",
        "json",
    ]);
    command.output().expect("the plan command runs")
}

#[test]
fn every_reclaimable_source_is_accepted_by_the_command_line() {
    for source in RECLAIMABLE {
        let output = plan_source(source);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("invalid value"),
            "--source {source} was rejected by the parser: {stderr}"
        );
        assert!(
            output.status.success(),
            "--source {source} failed: {stderr}"
        );
    }
}

#[test]
fn an_unknown_source_is_still_rejected() {
    let output = plan_source("not-a-tool");
    assert!(!output.status.success(), "an unknown source must fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid value"),
        "an unknown source must be reported as an invalid value"
    );
}

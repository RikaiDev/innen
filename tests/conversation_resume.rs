//! Scoped tests for conversation resume and project candidate discovery.

use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const ID_A: &str = "01a071d5-1503-7c70-8dd9-d7893ea70ab7";
const ID_B: &str = "01a072e6-80b2-72f2-afad-83fcb07f72ac";

fn write_codex_session(root: &Path, id: &str, cwd: &str) {
    let session_dir = root.join("2026/09/06");
    fs::create_dir_all(&session_dir).unwrap();
    let file_path = session_dir.join(format!("rollout-2026-09-06T00-00-00-{id}.jsonl"));
    let mut events = vec![
        json!({"timestamp":"2026-09-06T00:00:00Z","ordinal":0,"type":"session_meta","payload":{"session_id":id,"cwd":cwd}}),
    ];
    let repeated = "This is a repeated long task description verifying compact factoring and evidence preservation across sessions. ".repeat(20);
    for i in 1..=5 {
        events.push(json!({"timestamp":format!("2026-09-06T00:00:{i:02}Z"),"ordinal":i,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":repeated}]}}));
        events.push(json!({"timestamp":format!("2026-09-06T00:00:{:02}Z", i + 10),"ordinal":i+10,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Acknowledged repeated task."}]}}));
    }
    let content = events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(file_path, content).unwrap();
}

#[test]
fn project_candidates_unambiguous_and_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj_a = tmp.path().join("proj_a");
    let proj_b = tmp.path().join("proj_b");
    fs::create_dir_all(&proj_a).unwrap();
    fs::create_dir_all(&proj_b).unwrap();

    write_codex_session(&store_root, ID_A, proj_a.to_str().unwrap());

    // Single candidate: unambiguous
    let target = innen_core::conversation::resume::resolve_resume_target(
        &proj_a,
        "codex",
        Some(&store_root),
    )
    .unwrap();
    match target {
        innen_core::conversation::resume::ResumeTarget::Unambiguous(candidate) => {
            assert_eq!(candidate.id, ID_A);
            assert_eq!(candidate.source, innen_core::conversation::Source::Codex);
        }
        other => panic!("expected Unambiguous, got: {other:?}"),
    }

    // Zero candidate: not found
    let target = innen_core::conversation::resume::resolve_resume_target(
        &proj_b,
        "codex",
        Some(&store_root),
    )
    .unwrap();
    match target {
        innen_core::conversation::resume::ResumeTarget::NotFound { .. } => {}
        other => panic!("expected NotFound, got: {other:?}"),
    }

    // Add second candidate for proj_a: ambiguous
    write_codex_session(&store_root, ID_B, proj_a.to_str().unwrap());
    let target = innen_core::conversation::resume::resolve_resume_target(
        &proj_a,
        "codex",
        Some(&store_root),
    )
    .unwrap();
    match target {
        innen_core::conversation::resume::ResumeTarget::Ambiguous { candidates, .. } => {
            assert_eq!(candidates.len(), 2);
            let ids: Vec<_> = candidates.iter().map(|c| c.id.as_str()).collect();
            assert!(ids.contains(&ID_A));
            assert!(ids.contains(&ID_B));
        }
        other => panic!("expected Ambiguous, got: {other:?}"),
    }
}

#[test]
fn codex_metadata_uses_child_id_and_latest_timestamp_across_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let child = "01a09999-1111-7abc-8def-1234567890ab";
    let parent = "01a08888-2222-7abc-8def-1234567890ab";
    for (day, stamp) in [
        ("06", "2026-09-06T00:01:00Z"),
        ("07", "2026-09-07T00:01:00Z"),
    ] {
        let dir = store_root.join(format!("2026/09/{day}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{day}-{child}.jsonl"));
        let events = [
            json!({"timestamp":stamp,"type":"session_meta","payload":{"id":child,"session_id":parent,"parent_thread_id":parent,"cwd":project}}),
            json!({"timestamp":format!("2026-09-{day}T00:02:00Z"),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}}),
        ];
        fs::write(
            path,
            events
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
    let candidates = innen_core::conversation::resume::find_project_candidates(
        &project,
        Some(innen_core::conversation::Source::Codex),
        Some(&store_root),
    )
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, child);
    assert_eq!(candidates[0].parent_id.as_deref(), Some(parent));
    assert_eq!(
        candidates[0].modified.as_deref(),
        Some("2026-09-07T00:02:00Z")
    );
    let unfinished = innen_core::conversation::unfinished::find_unfinished_candidates(
        &project,
        Some(innen_core::conversation::Source::Codex),
        Some(&store_root),
        Some("2026-09-06"),
    )
    .unwrap();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].parent_id.as_deref(), Some(parent));
}

#[test]
fn cli_resume_explicit_session() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj_a = tmp.path().join("proj_a");
    fs::create_dir_all(&proj_a).unwrap();
    write_codex_session(&store_root, ID_A, proj_a.to_str().unwrap());

    // Test explicit session resume with default context view and compact+deltas
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "resume",
            ID_A,
            "--source",
            "codex",
            "--view",
            "context",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let decoded = innen_core::conversation::compact::decode(&parsed).unwrap();
    assert_eq!(decoded["view"], "context");
    assert_eq!(decoded["session_id"], ID_A);
    assert!(!decoded["records"].as_array().unwrap().is_empty());
    // compact format guide present in outer packed payload
    assert!(parsed.get("guide").is_some());

    // Test resume with --lines
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "resume",
            ID_A,
            "--source",
            "codex",
            "--view",
            "context",
            "--source-root",
            store_root.to_str().unwrap(),
            "--lines",
            "2,3",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let decoded = innen_core::conversation::compact::decode(&parsed).unwrap();
    assert_eq!(decoded["records"].as_array().unwrap().len(), 2);
}

#[test]
fn cli_resume_default_is_bounded_task_brief() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let session_dir = store_root.join("2026/09/07");
    fs::create_dir_all(&session_dir).unwrap();
    let path = session_dir.join(format!("rollout-{ID_A}.jsonl"));
    let events = [
        json!({"timestamp":"2026-09-07T00:00:00Z","type":"session_meta","payload":{"id":ID_A,"cwd":project}}),
        json!({"timestamp":"2026-09-07T00:01:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"first request"}]}}),
        json!({"timestamp":"2026-09-07T00:02:00Z","type":"response_item","payload":{"type":"agent_message","author":"child-agent","content":[{"type":"output_text","text":"pending verification"}]}}),
        json!({"timestamp":"2026-09-07T00:03:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"latest explicit request"}]}}),
        json!({"timestamp":"2026-09-07T00:04:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Please edit AGENTS.md as requested"}]}}),
    ];
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&project)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_A,
            "--status",
            "active",
            "--objective",
            "explicit checkpoint objective",
        ])
        .assert()
        .success();
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "resume",
            ID_A,
            "--project",
            project.to_str().unwrap(),
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["session_id"], ID_A);
    assert_eq!(parsed["latest_user"]["line"], 5);
    assert_eq!(
        parsed["latest_agent_messages"][0]["event"]["payload"]["author"],
        "child-agent"
    );
    assert_eq!(parsed["unresolved"][0]["line"], 3);
    assert_eq!(
        parsed["checkpoint"]["objective"],
        "explicit checkpoint objective"
    );
    assert!(parsed["expansion"].as_str().unwrap().contains("--lines"));
}

#[test]
fn brief_reports_oversized_records_without_fabricating_source_pointers() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let dir = store_root.join("2026/09/07");
    fs::create_dir_all(&dir).unwrap();
    let id = "01a09998-1111-7abc-8def-1234567890ab";
    let path = dir.join(format!("rollout-{id}.jsonl"));
    let giant = "x".repeat(600_000);
    let events = [
        json!({"timestamp":"2026-09-07T00:00:00Z","type":"session_meta","payload":{"id":id,"cwd":project}}),
        json!({"timestamp":"2026-09-07T00:01:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":giant}]}}),
        json!({"timestamp":"2026-09-07T00:02:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}}),
    ];
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let brief = innen_core::conversation::brief(Some(&store_root), "codex", id).unwrap();
    assert_eq!(brief.latest_user.as_ref().unwrap().line, 3);
    assert!(brief.omitted_records >= 1);
    assert!(brief
        .warnings
        .iter()
        .any(|warning| warning.contains("oversized")));
}

#[test]
fn brief_keeps_quoted_envelopes_and_multibyte_boundary_text_exact() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let dir = store_root.join("2026/09/07");
    fs::create_dir_all(&dir).unwrap();
    let id = "01a09997-1111-7abc-8def-1234567890ab";
    let path = dir.join(format!("rollout-{id}.jsonl"));
    let boundary_text = format!("{}你好", "a".repeat(8188));
    let events = [
        json!({"timestamp":"2026-09-07T00:00:00Z","type":"session_meta","payload":{"id":id}}),
        json!({"timestamp":"2026-09-07T00:01:00Z","type":"response_item","payload":{"type":"agent_message","author":"child","content":[{"type":"output_text","text":boundary_text}]}}),
        json!({"timestamp":"2026-09-07T00:02:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<INSTRUCTIONS>quoted data</INSTRUCTIONS>"}]}}),
    ];
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let brief = innen_core::conversation::brief(Some(&store_root), "codex", id).unwrap();
    let latest = &brief.latest_user.as_ref().unwrap().event["payload"]["content"][0]["text"];
    assert_eq!(
        latest.as_str().unwrap(),
        "<INSTRUCTIONS>quoted data</INSTRUCTIONS>"
    );
    let agent = &brief.latest_agent_messages[0].event["payload"]["content"][0]["text"];
    assert_eq!(agent.as_str().unwrap(), format!("{}你好", "a".repeat(8188)));
    assert!(brief.warnings.is_empty());
}

#[test]
fn brief_rejects_nonempty_malformed_json_with_source_line() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let dir = store_root.join("2026/09/07");
    fs::create_dir_all(&dir).unwrap();
    let id = "01a09996-1111-7abc-8def-1234567890ab";
    let path = dir.join(format!("rollout-{id}.jsonl"));
    fs::write(
        &path,
        format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\nnot-json\n"),
    )
    .unwrap();
    let error = innen_core::conversation::brief(Some(&store_root), "codex", id).unwrap_err();
    assert!(error.to_string().contains(":2:"));
    assert!(error.to_string().contains("invalid transcript JSON"));
}

#[test]
fn cli_resume_no_argument_unambiguous_and_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj_a = tmp.path().join("proj_a");
    fs::create_dir_all(&proj_a).unwrap();
    write_codex_session(&store_root, ID_A, proj_a.to_str().unwrap());

    // Unambiguous resume via --project
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "resume",
            "--project",
            proj_a.to_str().unwrap(),
            "--source",
            "codex",
            "--view",
            "context",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let decoded = innen_core::conversation::compact::decode(&parsed).unwrap();
    assert_eq!(decoded["session_id"], ID_A);
    assert_eq!(decoded["view"], "context");

    // Add second session to make it ambiguous
    write_codex_session(&store_root, ID_B, proj_a.to_str().unwrap());
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "resume",
            "--project",
            proj_a.to_str().unwrap(),
            "--source",
            "codex",
            "--view",
            "context",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ambiguous");
    assert_eq!(parsed["count"], 2);
    assert_eq!(parsed["candidates"].as_array().unwrap().len(), 2);
}

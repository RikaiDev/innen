//! Integration tests for checkpoint, unfinished discovery, pickup, and privacy.

use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const ID_1: &str = "11111111-1111-4111-8111-111111111111";
const ID_2: &str = "22222222-2222-4222-8222-222222222222";
const ID_3: &str = "33333333-3333-4333-8333-333333333333";
const ID_4: &str = "44444444-4444-4444-8444-444444444444";
const ID_5: &str = "55555555-5555-4555-8555-555555555555";
const ID_6: &str = "66666666-6666-4666-8666-666666666666";
const ID_7: &str = "77777777-7777-4777-8777-777777777777";

fn recent_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

fn make_recent(events: &mut [Value]) {
    let timestamp = Value::String(recent_rfc3339());
    for event in events {
        if event.get("timestamp").is_some() {
            event["timestamp"] = timestamp.clone();
        }
        if event.get("created_at").is_some() {
            event["created_at"] = timestamp.clone();
        }
    }
}

fn write_codex_session(root: &Path, id: &str, cwd: &str, events: &[Value]) {
    let session_dir = root.join("2026/09/06");
    fs::create_dir_all(&session_dir).unwrap();
    let file_path = session_dir.join(format!("rollout-2026-09-06T00-00-00-{id}.jsonl"));
    let mut all_events = vec![
        json!({"timestamp":"2026-09-06T00:00:00Z","ordinal":0,"type":"session_meta","payload":{"session_id":id,"cwd":cwd}}),
    ];
    all_events.extend_from_slice(events);
    make_recent(&mut all_events);
    let content = all_events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(file_path, content).unwrap();
}

fn write_antigravity_session(root: &Path, id: &str, cwd: &str, events: &[Value]) {
    let session_dir = root.join(id).join(".system_generated/logs");
    fs::create_dir_all(&session_dir).unwrap();
    let file_path = session_dir.join("transcript.jsonl");
    let mut all_events = vec![
        json!({"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-06T00:00:00Z","content":format!("<user_information>\nWorkspace: {}\n</user_information>\n<USER_REQUEST>\nWork on project in {}\n</USER_REQUEST>", cwd, cwd)}),
    ];
    all_events.extend_from_slice(events);
    make_recent(&mut all_events);
    let content = all_events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(file_path, content).unwrap();
}

fn write_antigravity_session_no_workspace(root: &Path, id: &str, events: &[Value]) {
    let session_dir = root.join(id).join(".system_generated/logs");
    fs::create_dir_all(&session_dir).unwrap();
    let file_path = session_dir.join("transcript.jsonl");
    let mut all_events = vec![
        json!({"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-06T00:00:00Z","content":"<user_information>\nThe user does not have any active workspace.\n</user_information>"}),
    ];
    all_events.extend_from_slice(events);
    make_recent(&mut all_events);
    let content = all_events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(file_path, content).unwrap();
}

#[test]
fn checkpoint_record_and_append_only_history() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("my_project");
    fs::create_dir_all(&proj).unwrap();

    // 1. Record active checkpoint
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "checkpoint",
            "record",
            "--session",
            ID_1,
            "--source",
            "codex",
            "--status",
            "active",
            "--objective",
            "Refactor compiler frontend",
            "--completed",
            "Lexer complete,AST parsed",
            "--evidence",
            "Unit tests pass",
            "--blocker",
            "Need typechecker API",
            "--next-action",
            "Implement symbol table",
            "--verification",
            "cargo test --test parser",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "record active failed");
    let res: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(res["status"], "recorded");
    assert_eq!(res["checkpoint"]["session"], ID_1);
    assert_eq!(res["checkpoint"]["status"], "active");
    assert_eq!(
        res["checkpoint"]["completed_work"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Verify .innen/.gitignore was created and contains *
    let gitignore_path = proj.join(".innen/.gitignore");
    assert!(gitignore_path.exists());
    let gitignore_content = fs::read_to_string(gitignore_path).unwrap();
    assert!(gitignore_content.contains('*'));

    // Verify checkpoints.jsonl has 1 line
    let cp_file = proj.join(".innen/checkpoints.jsonl");
    assert!(cp_file.exists());
    let lines1: Vec<_> = fs::read_to_string(&cp_file)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(lines1.len(), 1);

    // 2. Update checkpoint to completed (append-only)
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "checkpoint",
            "record",
            "--session",
            ID_1,
            "--status",
            "completed",
            "--completed",
            "Symbol table done",
            "--next-action",
            "None, all done",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "record completed failed");

    // Verify checkpoints.jsonl now has 2 lines (history preserved!)
    let lines2: Vec<_> = fs::read_to_string(&cp_file)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(lines2.len(), 2);
    // Line 1 is still the old active checkpoint
    let first: Value = serde_json::from_str(&lines2[0]).unwrap();
    assert_eq!(first["status"], "active");
    // Line 2 is the new completed checkpoint
    let second: Value = serde_json::from_str(&lines2[1]).unwrap();
    assert_eq!(second["status"], "completed");

    // 3. Checkpoint show returns latest (completed)
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args(["--format", "json", "checkpoint", "show", ID_1])
        .output()
        .unwrap();
    assert!(output.status.success());
    let show: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(show["session"], ID_1);
    assert_eq!(show["status"], "completed");

    // 4. Human format for checkpoint show
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args(["--format", "human", "checkpoint", "show", ID_1])
        .output()
        .unwrap();
    assert!(output.status.success());
    let human = String::from_utf8_lossy(&output.stdout);
    assert!(human.contains(ID_1));
    assert!(human.contains("completed"));
    assert!(human.contains("Refactor compiler frontend"));
}

#[test]
fn unfinished_excludes_completed_and_includes_active() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // Session 1: has active checkpoint
    write_codex_session(
        &store_root,
        ID_1,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Implement feature"}]}}),
            json!({"timestamp":"2026-09-06T00:02:00Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Working on it"}]}}),
        ],
    );

    // Session 2: has completed checkpoint
    write_codex_session(
        &store_root,
        ID_2,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix bug"}]}}),
            json!({"timestamp":"2026-09-06T00:02:00Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixed"}]}}),
        ],
    );

    // Record checkpoints: ID_1 is active, ID_2 is completed
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_1,
            "--status",
            "active",
            "--objective",
            "Feature in progress",
        ])
        .assert()
        .success();

    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_2,
            "--status",
            "completed",
            "--objective",
            "Bug fix finished",
        ])
        .assert()
        .success();

    // Query unfinished candidates
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
    let candidates = parsed["candidates"].as_array().unwrap();

    // ID_1 must be present (active)
    assert!(candidates.iter().any(|c| c["id"] == ID_1));
    // ID_2 must be EXCLUDED (completed)
    assert!(!candidates.iter().any(|c| c["id"] == ID_2));

    let c1 = candidates.iter().find(|c| c["id"] == ID_1).unwrap();
    assert_eq!(c1["classification"], "candidate");
    assert_eq!(c1["confidence"], "high");
    let reason_codes: Vec<_> = c1["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(reason_codes.contains(&"checkpoint_active"));
}

#[test]
fn unfinished_detects_interrupted_and_unanswered_user_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // Session 1: interrupted/cancelled
    write_codex_session(
        &store_root,
        ID_1,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Run build"}]}}),
            json!({"timestamp":"2026-09-06T00:02:00Z","ordinal":2,"type":"event_msg","payload":{"type":"turn_aborted","reason":"interrupted"}}),
        ],
    );

    // Session 2: trailing unanswered user request
    write_codex_session(
        &store_root,
        ID_2,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Initial task"}]}}),
            json!({"timestamp":"2026-09-06T00:02:00Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}}),
            json!({"timestamp":"2026-09-06T00:03:00Z","ordinal":3,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Now run tests"}]}}),
        ],
    );

    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let candidates = parsed["candidates"].as_array().unwrap();

    let c1 = candidates.iter().find(|c| c["id"] == ID_1).unwrap();
    let c1_reasons: Vec<_> = c1["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(c1_reasons.contains(&"interrupted_or_cancelled"));

    let c2 = candidates.iter().find(|c| c["id"] == ID_2).unwrap();
    let c2_reasons: Vec<_> = c2["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(c2_reasons.contains(&"trailing_user_request_unanswered"));
}

#[test]
fn pickup_unambiguous_selects_and_emits_continuation() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // Exactly 1 candidate session
    write_codex_session(
        &store_root,
        ID_1,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Build backend"}]}}),
            json!({"timestamp":"2026-09-06T00:02:00Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"In progress"}]}}),
        ],
    );

    // Record checkpoint
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_1,
            "--status",
            "active",
            "--objective",
            "Build robust API backend",
            "--completed",
            "Database schema",
            "--next-action",
            "Add authentication endpoint",
            "--verification",
            "cargo test auth",
        ])
        .assert()
        .success();

    // Pickup unambiguous
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "pickup",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "picked_up");
    assert_eq!(parsed["session_id"], ID_1);
    assert_eq!(parsed["next_action"], "Add authentication endpoint");
    assert_eq!(parsed["verification"], "cargo test auth");
    assert!(parsed["resume_command"].as_str().unwrap().contains(ID_1));
    assert!(parsed["evidence_pointers"].is_object());

    // Test human output for pickup
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "human",
            "pickup",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let human = String::from_utf8_lossy(&output.stdout);
    assert!(human.contains("Picked up session:"));
    assert!(human.contains(ID_1));
    assert!(human.contains("Add authentication endpoint"));
    assert!(human.contains("cargo test auth"));
}

#[test]
fn pickup_ambiguous_returns_candidate_list() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // Two candidates for the same project
    write_codex_session(
        &store_root,
        ID_1,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Task 1"}]}}),
        ],
    );
    write_codex_session(
        &store_root,
        ID_2,
        proj.to_str().unwrap(),
        &[
            json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Task 2"}]}}),
        ],
    );

    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "pickup",
            "--source",
            "codex",
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

#[test]
fn antigravity_candidate_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("brain");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    write_antigravity_session(
        &store_root,
        ID_3,
        proj.to_str().unwrap(),
        &[
            json!({"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-06T00:01:00Z","content":"Executing command","tool_calls":[{"name":"run_command","args":{"Cwd":proj.to_str().unwrap()}}]}),
            json!({"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-06T00:02:00Z","content":"<USER_REQUEST>Next step</USER_REQUEST>"}),
        ],
    );

    // Unfinished discovery for agy
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "agy",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agy unfinished failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
    let candidates = parsed["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["id"], ID_3);
    assert_eq!(candidates[0]["source"], "antigravity");
    assert_eq!(candidates[0]["classification"], "candidate");
}

#[test]
fn date_boundary_filters_older_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // Session modified in past (2020)
    let session_dir = store_root.join("2020/01/01");
    fs::create_dir_all(&session_dir).unwrap();
    let file_path = session_dir.join(format!("rollout-2020-01-01T00-00-00-{ID_1}.jsonl"));
    let events = [
        json!({"timestamp":"2020-01-01T00:00:00Z","ordinal":0,"type":"session_meta","payload":{"session_id":ID_1,"cwd":proj.to_str().unwrap()}}),
        json!({"timestamp":"2020-01-01T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Old task"}]}}),
    ];
    fs::write(
        file_path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    // Default since (yesterday+today) must filter out the 2020 session
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 0);

    // Providing --since 2019-01-01 should include it
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
            "--since",
            "2019-01-01",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["candidates"][0]["id"], ID_1);
}

#[test]
fn all_projects_portfolio_discovery_provenance_and_explicit_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let codex_root = tmp.path().join("codex_sessions");
    let agy_root = tmp.path().join("agy_brain");
    let proj_a = tmp.path().join("project_alpha");
    let proj_b = tmp.path().join("project_beta");
    fs::create_dir_all(&proj_a).unwrap();
    fs::create_dir_all(&proj_b).unwrap();

    let user_event = [
        json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Task in progress"}]}}),
    ];

    // Session 1: Codex in project_alpha
    write_codex_session(&codex_root, ID_1, proj_a.to_str().unwrap(), &user_event);
    // Session 2: Codex in project_beta
    write_codex_session(&codex_root, ID_2, proj_b.to_str().unwrap(), &user_event);
    // Session 3: Antigravity in project_alpha
    write_antigravity_session(
        &agy_root,
        ID_3,
        proj_a.to_str().unwrap(),
        &[
            json!({"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-06T00:01:00Z","content":"Unfinished task alpha"}),
        ],
    );
    // Session 4: Antigravity with NO active workspace
    write_antigravity_session_no_workspace(
        &agy_root,
        ID_4,
        &[
            json!({"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-06T00:01:00Z","content":"Unfinished general query"}),
        ],
    );

    // 1. Discover unfinished candidates across all projects for Codex
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "json",
            "unfinished",
            "--all-projects",
            "--source",
            "codex",
            "--source-root",
            codex_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "codex all-projects failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["all_projects"], true);
    assert_eq!(parsed["count"], 2);

    let candidates = parsed["candidates"].as_array().unwrap();
    let c1 = candidates.iter().find(|c| c["id"] == ID_1).unwrap();
    let c2 = candidates.iter().find(|c| c["id"] == ID_2).unwrap();
    assert_eq!(c1["project"], proj_a.to_str().unwrap());
    assert_eq!(c2["project"], proj_b.to_str().unwrap());

    // 2. Discover unfinished candidates across all projects for Antigravity
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "json",
            "unfinished",
            "--all-projects",
            "--source",
            "agy",
            "--source-root",
            agy_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agy all-projects failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["all_projects"], true);
    assert_eq!(parsed["count"], 2);

    let candidates = parsed["candidates"].as_array().unwrap();
    let c3 = candidates.iter().find(|c| c["id"] == ID_3).unwrap();
    let c4 = candidates.iter().find(|c| c["id"] == ID_4).unwrap();
    assert_eq!(c3["project"], proj_a.to_str().unwrap());
    assert_eq!(c4["project"], "unknown");

    // 3. Verify Human output format for all-projects
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "human",
            "unfinished",
            "--all-projects",
            "--source",
            "agy",
            "--source-root",
            agy_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let human = String::from_utf8_lossy(&output.stdout);
    assert!(human.contains("Unfinished candidate sessions across all projects:"));
    assert!(human.contains("project: unknown"));
    assert!(human.contains(proj_a.to_str().unwrap()));
}

#[test]
fn all_projects_preserves_per_project_checkpoints_and_scoped_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj_a = tmp.path().join("project_alpha");
    let proj_b = tmp.path().join("project_beta");
    fs::create_dir_all(&proj_a).unwrap();
    fs::create_dir_all(&proj_b).unwrap();

    let user_event = [
        json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Implement feature"}]}}),
    ];

    write_codex_session(&store_root, ID_1, proj_a.to_str().unwrap(), &user_event);
    write_codex_session(&store_root, ID_2, proj_b.to_str().unwrap(), &user_event);

    // Record checkpoint in proj_a for ID_1: COMPLETED
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj_a)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_1,
            "--status",
            "completed",
            "--objective",
            "Finished alpha task",
        ])
        .assert()
        .success();

    // Record checkpoint in proj_b for ID_2: ACTIVE
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj_b)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_2,
            "--status",
            "active",
            "--objective",
            "Beta task in flight",
            "--next-action",
            "Run beta tests",
        ])
        .assert()
        .success();

    // 1. All-projects portfolio discovery:
    // ID_1 (completed in proj_a) MUST BE EXCLUDED
    // ID_2 (active in proj_b) MUST BE INCLUDED with active checkpoint attached
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "json",
            "unfinished",
            "--all-projects",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
    let candidates = parsed["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["id"], ID_2);
    assert_eq!(candidates[0]["project"], proj_b.to_str().unwrap());
    assert_eq!(candidates[0]["checkpoint"]["status"], "active");
    assert_eq!(
        candidates[0]["checkpoint"]["objective"],
        "Beta task in flight"
    );

    // 2. Project-scoped discovery in proj_a:
    // ID_1 is completed -> 0 candidates in proj_a
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj_a)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 0);

    // 3. Project-scoped discovery in proj_b:
    // ID_2 is active -> 1 candidate in proj_b
    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj_b)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["candidates"][0]["id"], ID_2);
}

#[test]
fn all_projects_pickup_unambiguous_and_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("sessions");
    let proj_a = tmp.path().join("project_alpha");
    let proj_b = tmp.path().join("project_beta");
    fs::create_dir_all(&proj_a).unwrap();
    fs::create_dir_all(&proj_b).unwrap();

    let user_event = [
        json!({"timestamp":"2026-09-06T00:01:00Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Task 1"}]}}),
    ];

    write_codex_session(&store_root, ID_1, proj_a.to_str().unwrap(), &user_event);
    write_codex_session(&store_root, ID_2, proj_b.to_str().unwrap(), &user_event);

    // 1. Multiple candidates across projects -> ambiguous pickup
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "json",
            "pickup",
            "--all-projects",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "ambiguous");
    assert_eq!(parsed["count"], 2);

    // Mark ID_2 completed in proj_b
    Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj_b)
        .args([
            "checkpoint",
            "record",
            "--session",
            ID_2,
            "--status",
            "completed",
            "--objective",
            "Done",
        ])
        .assert()
        .success();

    // 2. Exactly 1 unfinished remains across projects -> picked_up
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--format",
            "json",
            "pickup",
            "--all-projects",
            "--source",
            "codex",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "picked_up");
    assert_eq!(parsed["session_id"], ID_1);
    assert_eq!(parsed["project"], proj_a.to_str().unwrap());
    assert!(parsed["resume_command"].as_str().unwrap().contains(ID_1));
}

fn write_copilot_session(root: &Path, id: &str, cwd: &str, events: &[Value]) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("workspace.yaml"),
        format!("productVersion: 1.0.0\ncwd: {cwd}\n"),
    )
    .unwrap();
    let content = events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join("events.jsonl"), content).unwrap();
}

fn write_grok_session(
    root: &Path,
    encoded_project: &str,
    id: &str,
    chat: &[Value],
    events: &[Value],
    updates: &[Value],
) {
    let dir = root.join(encoded_project).join(id);
    fs::create_dir_all(&dir).unwrap();
    let lines = |stream: &[Value]| {
        stream
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    };
    fs::write(dir.join("chat_history.jsonl"), lines(chat)).unwrap();
    fs::write(dir.join("events.jsonl"), lines(events)).unwrap();
    if !updates.is_empty() {
        fs::write(dir.join("updates.jsonl"), lines(updates)).unwrap();
    }
}

fn reason_codes(candidate: &Value) -> Vec<String> {
    candidate["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap().to_string())
        .collect()
}

fn candidate_by_id<'a>(candidates: &'a [Value], id: &str) -> &'a Value {
    candidates
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("candidate {id} missing in {:?}", candidates.len()))
}

#[test]
fn copilot_candidate_discovery_workspaces_and_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("session-state");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    let cwd = proj.to_str().unwrap();

    // Trailing user request after a completed turn: kept, high confidence.
    write_copilot_session(
        &store_root,
        ID_5,
        cwd,
        &[
            json!({"type":"assistant.turn_start"}),
            json!({"type":"user.message","data":{"text":"First ask"}}),
            json!({"type":"assistant.message","data":{"text":"Answer"}}),
            json!({"type":"assistant.turn_end"}),
            json!({"type":"user.message","data":{"text":"Follow-up"}}),
        ],
    );
    // A cleanly ended turn is the turn-level marker only: discovery keeps
    // the candidate and the marker surfaces at low confidence.
    write_copilot_session(
        &store_root,
        ID_6,
        cwd,
        &[
            json!({"type":"assistant.turn_start"}),
            json!({"type":"user.message","data":{"text":"Second ask"}}),
            json!({"type":"assistant.message","data":{"text":"Answer"}}),
            json!({"type":"assistant.turn_end"}),
        ],
    );
    // `success:false` stays an unresolved failure; the turn ending after it
    // does not close it. The request is answered first so the trailing
    // signal is separate (covered by ID_5).
    write_copilot_session(
        &store_root,
        ID_7,
        cwd,
        &[
            json!({"type":"assistant.turn_start"}),
            json!({"type":"user.message","data":{"text":"Third ask"}}),
            json!({"type":"assistant.message","data":{"text":"Trying"}}),
            json!({"type":"tool.execution_complete","data":{"success":false}}),
            json!({"type":"assistant.turn_end"}),
        ],
    );
    // A scaffold directory without an animated conversation is not a
    // candidate (scanner), and workspace.yaml locates the project.
    let scaffold = store_root.join("88888888-8888-4888-8888-888888888888");
    fs::create_dir_all(&scaffold).unwrap();
    fs::write(scaffold.join("workspace.yaml"), format!("cwd: {cwd}\n")).unwrap();

    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "copilot",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 3);
    let candidates = parsed["candidates"].as_array().unwrap();
    // Scoped discovery reports the process cwd, which the OS resolves
    // physically (macOS /var -> /private/var); the store keeps the symlinked
    // form for matching, the report keeps the process view. Canonicalize
    // compares the two forms.
    let expected = proj
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(candidates
        .iter()
        .all(|c| c["source"] == "copilot" && c["project"].as_str() == Some(expected.as_str())));

    let trailing = candidate_by_id(candidates, ID_5);
    assert!(reason_codes(trailing).contains(&"trailing_user_request_unanswered".into()));
    assert_eq!(trailing["confidence"], "high");

    let completed = candidate_by_id(candidates, ID_6);
    let codes = reason_codes(completed);
    assert!(codes.contains(&"explicit_turn_completed".into()), "{codes:?}");
    assert_eq!(completed["confidence"], "low");

    let failed = candidate_by_id(candidates, ID_7);
    assert!(reason_codes(failed).contains(&"unresolved_tool_failure".into()));
    assert_eq!(failed["confidence"], "medium");
}

#[test]
fn grok_candidate_discovery_stream_evidence_and_unconsumed_acp_channel() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("grok-sessions");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    let encoded = proj.to_string_lossy().replace('/', "%2F");

    // Trailing user request + the last turn ending with an error outcome.
    // A `reasoning` record after it is not an answer.
    write_grok_session(
        &store_root,
        &encoded,
        ID_5,
        &[
            json!({"type":"user","content":"Build the thing"}),
            json!({"type":"assistant","content":"Working on it"}),
            json!({"type":"user","content":"Also fix the tests"}),
            json!({"type":"reasoning","content":"thinking"}),
        ],
        &[
            json!({"type":"tool_started"}),
            json!({"type":"tool_completed","outcome":"success"}),
            json!({"type":"loop_started"}),
            json!({"type":"turn_ended","outcome":"error"}),
        ],
        &[],
    );
    // A succeeded turn is the marker; the ACP chunk in updates.jsonl is not
    // consumed, so no trailing reason appears despite the late chunk.
    write_grok_session(
        &store_root,
        &encoded,
        ID_6,
        &[
            json!({"type":"user","content":"One shot"}),
            json!({"type":"assistant","content":"Done"}),
        ],
        &[
            json!({"type":"loop_started"}),
            json!({"type":"tool_completed","outcome":"success"}),
            json!({"type":"turn_ended","outcome":"success"}),
        ],
        &[json!({"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"text":"late ACP ask"}}}})],
    );
    // A failed tool outcome after the completed turn stays open.
    write_grok_session(
        &store_root,
        &encoded,
        ID_7,
        &[
            json!({"type":"user","content":"Shot"}),
            json!({"type":"assistant","content":"Done"}),
        ],
        &[
            json!({"type":"turn_ended","outcome":"success"}),
            json!({"type":"tool_completed","outcome":"error"}),
        ],
        &[],
    );

    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "grok",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 3);
    let candidates = parsed["candidates"].as_array().unwrap();
    // Same physical-cwd rule as the copilot test above.
    let expected = proj
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(candidates
        .iter()
        .all(|c| c["source"] == "grok" && c["project"].as_str() == Some(expected.as_str())));

    let unfinished = candidate_by_id(candidates, ID_5);
    let codes = reason_codes(unfinished);
    assert!(
        codes.contains(&"trailing_user_request_unanswered".into())
            && codes.contains(&"interrupted_or_cancelled".into()),
        "{codes:?}"
    );
    assert_eq!(unfinished["confidence"], "high");

    let completed = candidate_by_id(candidates, ID_6);
    let codes = reason_codes(completed);
    assert_eq!(
        codes,
        vec!["explicit_turn_completed".to_string()],
        "{codes:?}"
    );
    assert_eq!(completed["confidence"], "low");

    let failed = candidate_by_id(candidates, ID_7);
    assert!(reason_codes(failed).contains(&"unresolved_tool_failure".into()));
    assert_eq!(failed["confidence"], "medium");

    // Coverage speaks with a fixed vocabulary per store row.
    for row in parsed["coverage"].as_array().unwrap() {
        let status = row["status"].as_str().unwrap();
        assert!(
            ["scanned", "missing", "skipped", "not-supported"].contains(&status),
            "{row:?}"
        );
    }
}

#[test]
fn mcode_candidate_discovery_and_status_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let store_root = tmp.path().join("minimax");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    let cwd = proj.to_str().unwrap();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let db = store_root.join("sqlite").join("runtime-state.sqlite");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let status = std::process::Command::new("sqlite3")
        .arg(&db)
        .arg(format!(
            "CREATE TABLE local_runtime_sessions(session_id TEXT PRIMARY KEY, workspace_dir TEXT, project_workspace_dir TEXT, updated_at_ms INTEGER, history_relative_dir TEXT, parent_session_id TEXT, archived INTEGER, visibility TEXT, status TEXT, error_message TEXT); \
             INSERT INTO local_runtime_sessions VALUES('mvs_aborted001','{cwd}','{cwd}',{now_ms},'day1/dir1','','0','visible','aborted',''); \
             INSERT INTO local_runtime_sessions VALUES('mvs_error0001','{cwd}','{cwd}',{now_ms},'day1/dir2','','0','visible','error','boom'); \
             INSERT INTO local_runtime_sessions VALUES('mvs_idle0001','{cwd}','{cwd}',{now_ms},'','','0','visible','idle',''); \
             INSERT INTO local_runtime_sessions VALUES('mvs_archived1','{cwd}','{cwd}',{now_ms},'day1/dir3','','1','visible','aborted','');"
        ))
        .status()
        .unwrap();
    assert!(status.success());
    for dir in ["day1/dir1", "day1/dir2", "day1/dir3"] {
        let transcript = store_root.join("sessions").join(dir);
        fs::create_dir_all(&transcript).unwrap();
        fs::write(
            transcript.join("messages.jsonl"),
            "{\"type\":\"message\",\"role\":\"user\",\"content\":\"work\"}\n",
        )
        .unwrap();
    }

    let output = Command::cargo_bin("innen")
        .unwrap()
        .current_dir(&proj)
        .args([
            "--format",
            "json",
            "unfinished",
            "--source",
            "mcode",
            "--source-root",
            store_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["count"], 3);
    let candidates = parsed["candidates"].as_array().unwrap();
    // Same physical-cwd rule as the copilot test above.
    let expected = proj
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(candidates
        .iter()
        .all(|c| c["source"] == "mcode" && c["project"].as_str() == Some(expected.as_str())));

    let aborted = candidate_by_id(candidates, "mvs_aborted001");
    assert!(reason_codes(aborted).contains(&"interrupted_or_cancelled".into()));
    assert_eq!(aborted["confidence"], "high");

    let errored = candidate_by_id(candidates, "mvs_error0001");
    let failure = errored["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == "unresolved_tool_failure")
        .cloned()
        .unwrap();
    assert!(failure["detail"].as_str().unwrap().contains("boom"));
    assert_eq!(errored["confidence"], "medium");

    // No completed status exists in the runtime database: an idle session
    // stays a quiet low-confidence recent-activity candidate.
    let idle = candidate_by_id(candidates, "mvs_idle0001");
    let codes = reason_codes(idle);
    assert!(
        codes.contains(&"recent_activity_uncheckpointed".into()) && codes.len() == 1,
        "{codes:?}"
    );
    assert_eq!(idle["confidence"], "low");
}

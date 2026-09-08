use assert_cmd::Command;
use innen_core::ids::sha256_hex;
use serde_json::{json, Value};

fn decision(id: &str, project: &str, action: &str, statement: &str) -> Value {
    json!({"id":id,"project":project,"task_id":"task:long","actions":[action],
        "observed_at":"2026-01-01T00:00:00Z",
        "statement":statement,"rationale":"Source-backed decision for this task",
        "conditions":{},"sources":[{"id":format!("source:{id}"),"sha256":sha256_hex(statement.as_bytes())}],
        "supersedes":[],"depends_on":[],"lessons":[],"reopen_when":[]})
}

fn request(action: &str) -> Value {
    let mut events = Vec::new();
    for (key, old, new, lesson) in [
        (
            "cleanup",
            "Delete every build cache immediately",
            "Check active consumers atomically before cleaning",
            "A printed process check did not stop deletion of an active dependency tree",
        ),
        (
            "documentation",
            "Remove every token efficiency claim",
            "Keep token efficiency purpose and report matched measurement conditions",
            "Removing the product objective was an overcorrection",
        ),
        (
            "release",
            "Publish all working research records",
            "Publish product code and public fixtures only",
            "Private operational history must remain outside public release history",
        ),
    ] {
        let old_id = format!("{key}-old");
        let mut previous = decision(
            &old_id,
            "project:tool",
            key,
            &format!("{old}. {}", "Historical investigation detail. ".repeat(80)),
        );
        previous["lessons"] = json!([lesson]);
        let mut current = decision(&format!("{key}-current"), "project:tool", key, new);
        current["observed_at"] = json!("2026-01-02T00:00:00Z");
        current["supersedes"] = json!([old_id]);
        events.push(previous);
        events.push(current);
    }
    // Sort chronological history; equal timestamps retain deterministic order.
    events.sort_by_key(|v| v["observed_at"].as_str().unwrap().to_owned());
    events.push(decision(
        "foreign",
        "project:other",
        action,
        "UNRELATED_PRIVATE_PROJECT",
    ));
    events.last_mut().unwrap()["observed_at"] = json!("2026-01-03T00:00:00Z");
    json!({"stable_prefix":"Preserve authority and counterevidence","task":{"id":"task:long","state":"active","project":"project:tool","action":action,"conditions":{}},"budget_tokens":4096,"events":events})
}

fn invoke(value: &Value, extra: &[&str]) -> std::process::Output {
    Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-prepare", "--file", "-"])
        .args(extra)
        .write_stdin(value.to_string())
        .output()
        .unwrap()
}

#[test]
fn three_decision_corrections_keep_lessons_without_obsolete_instructions() {
    for action in ["cleanup", "documentation", "release"] {
        let r = request(action);
        let out = invoke(&r, &["--packet-only"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let packet = String::from_utf8(out.stdout).unwrap();
        assert!(packet.contains(&format!("{action}-current")));
        assert!(!packet.contains("Historical investigation detail"));
        assert!(!packet.contains("UNRELATED_PRIVATE_PROJECT"));
        let expected = match action {
            "cleanup" => "A printed process check did not stop deletion",
            "documentation" => "Removing the product objective was an overcorrection",
            _ => "Private operational history must remain outside public release history",
        };
        assert!(packet.contains(expected));
        let full = invoke(&r, &[]);
        assert!(full.status.success());
        let receipt: Value = serde_json::from_slice(&full.stdout).unwrap();
        let same_task: Vec<Value> = r["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e["project"] == "project:tool"
                    && e["actions"].as_array().unwrap().iter().any(|a| a == action)
            })
            .cloned()
            .collect();
        let baseline =
            json!({"stable_prefix":r["stable_prefix"],"task":r["task"],"events":same_task});
        let before = innen_core::conversation::grammar::tokens(&baseline).unwrap();
        let after = receipt["prepared"]["after_reference_tokens"]
            .as_u64()
            .unwrap() as usize;
        assert!(
            after < before,
            "causal summary should remove long obsolete statement while preserving lesson"
        );
        eprintln!("history benchmark {action}: {before} -> {after} reference tokens");
        let hash = receipt["history_source_sha256"].as_str().unwrap();
        let expanded = Command::cargo_bin("innen")
            .unwrap()
            .args([
                "middleware",
                "history-expand",
                "--file",
                "-",
                "--expect-source-sha256",
                hash,
                "--ids",
                &format!("{action}-old"),
            ])
            .write_stdin(r.to_string())
            .output()
            .unwrap();
        assert!(expanded.status.success());
        assert!(String::from_utf8(expanded.stdout)
            .unwrap()
            .contains("Historical investigation detail"));
    }
}

#[test]
fn unrelated_task_and_required_budget_failure_produce_no_packet() {
    let mut r = request("missing-action");
    let out = invoke(&r, &["--packet-only"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    r = request("release");
    r["budget_tokens"] = json!(1);
    let out = invoke(&r, &["--packet-only"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn forged_reference_and_cross_project_replacement_are_rejected() {
    let mut r = request("release");
    r["events"][0]["sources"][0]["sha256"] = json!("not-a-hash");
    assert!(!invoke(&r, &[]).status.success());
    let mut r = request("release");
    let n = r["events"].as_array().unwrap().len() - 1;
    r["events"][n]["supersedes"] = json!(["release-current"]);
    assert!(!invoke(&r, &[]).status.success());
}

#[test]
fn missing_conditions_withhold_active_statement_and_packet_dispatch() {
    let mut r = request("release");
    let current = r["events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == "release-current")
        .unwrap();
    current["conditions"] = json!({"evidence_review":"passed"});
    let out = invoke(&r, &[]);
    assert_eq!(out.status.code(), Some(2));
    let receipt: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(receipt["resolution"], "unknown");
    assert!(!receipt["prepared"]["packet"]
        .to_string()
        .contains("Publish product code and public fixtures only"));
    let packet = invoke(&r, &["--packet-only"]);
    assert_eq!(packet.status.code(), Some(2));
    assert!(packet.stdout.is_empty());
}

#[test]
fn long_task_crosses_sessions_and_dates_without_importing_urgent_task() {
    let mut r = request("cleanup");
    for event in r["events"].as_array_mut().unwrap() {
        event["sources"][0]["session_id"] =
            json!(if event["id"].as_str().unwrap().ends_with("old") {
                "session:january"
            } else {
                "session:february"
            });
        if event["id"].as_str().unwrap().ends_with("current") {
            event["observed_at"] = json!("2026-02-02T00:00:00Z");
        }
    }
    let mut urgent = decision(
        "urgent-decision",
        "project:tool",
        "cleanup",
        "UNRELATED_URGENT_TASK",
    );
    urgent["task_id"] = json!("task:urgent");
    urgent["observed_at"] = json!("2026-03-02T00:00:00Z");
    r["events"].as_array_mut().unwrap().push(urgent);
    r["events"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|v| v["observed_at"].as_str().unwrap().to_owned());
    let a = invoke(&r, &["--packet-only"]);
    assert!(a.status.success());
    let text = String::from_utf8(a.stdout).unwrap();
    assert!(text.contains("session:january") && text.contains("session:february"));
    assert!(!text.contains("UNRELATED_URGENT_TASK"));
    r["task"]["state"] = json!("paused");
    let paused = invoke(&r, &["--packet-only"]);
    assert!(!paused.status.success());
    assert!(paused.stdout.is_empty());
    r["task"]["state"] = json!("active");
    let resumed = invoke(&r, &["--packet-only"]);
    assert!(resumed.status.success());
    assert_eq!(String::from_utf8(resumed.stdout).unwrap(), text);
}

#[test]
fn session_start_and_compact_hook_emit_only_for_bound_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("history.json");
    std::fs::write(&file, request("cleanup").to_string()).unwrap();
    for source in ["startup", "resume", "compact"] {
        let event = json!({"hook_event_name":"SessionStart","session_id":"fixture-session","source":source,"cwd":temp.path()});
        let output = Command::cargo_bin("innen")
            .unwrap()
            .args(["middleware", "history-hook", "--file"])
            .arg(&file)
            .arg("--cwd")
            .arg(temp.path())
            .write_stdin(event.to_string())
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let context = value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("cleanup-current"));
        assert!(!context.contains("Historical investigation detail"));
    }
    let other = tempfile::tempdir().unwrap();
    let event = json!({"hook_event_name":"SessionStart","session_id":"fixture-session","source":"compact","cwd":other.path()});
    let output = Command::cargo_bin("innen")
        .unwrap()
        .args(["middleware", "history-hook", "--file"])
        .arg(&file)
        .arg("--cwd")
        .arg(temp.path())
        .write_stdin(event.to_string())
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
}

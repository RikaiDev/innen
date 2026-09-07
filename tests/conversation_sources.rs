//! Synthetic source contracts, separate from local-store verification.
use serde_json::{json, Value};
use std::path::Path;

const ID: &str = "12345678-1234-4567-89ab-123456789abc";

fn write(root: &Path, relative: &str, data: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

fn page(root: &Path, source: &str, view: &str) -> Value {
    serde_json::to_value(
        innen_core::conversation::read(Some(root), source, ID, view, 0, 20).unwrap(),
    )
    .unwrap()
}

#[test]
fn index_links_only_explicit_unique_in_page_call_ids() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"保留失敗，不要發布。".repeat(30)}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"pair","name":"check","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"pair","output":"FAIL: not approved"}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"duplicate","name":"one","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"duplicate","name":"two","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"duplicate","output":"unknown"}}),
    ];
    write(
        root.path(),
        &format!("{ID}.jsonl"),
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let index = page(root.path(), "codex", "index");
    assert_eq!(index["records"][0]["event"]["preview_truncated"], true);
    assert_eq!(
        index["records"][0]["event"]["preview"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        60
    );
    assert_eq!(index["records"][2]["event"]["call_line"], 2);
    assert!(index["records"][2]["event"].get("call_id").is_none());
    assert_eq!(index["records"][5]["event"]["call_id"], "duplicate");
    let tail =
        innen_core::conversation::read(Some(root.path()), "codex", ID, "index", 2, 1).unwrap();
    assert_eq!(tail.records[0].event["call_id"], "pair");
    let raw = innen_core::conversation::read_lines(Some(root.path()), "codex", ID, &[3]).unwrap();
    assert_eq!(raw.records[0].event, events[2]);
}

#[test]
fn context_keeps_media_cues_unknown_events_and_negative_output_despite_exit_zero() {
    let root = tempfile::tempdir().unwrap();
    let body = format!(
        "FAIL: not accepted\n{}\nEND: do not publish",
        "middle evidence\n".repeat(30)
    );
    let events = vec![
        json!({"type":"response_item","status":"cancelled","truncated_fields":["payload.content"],"payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,YWJj"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"c1","name":"check","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":format!("Chunk ID: abc\nWall time: 0.1 seconds\nProcess exited with code 0\nOriginal token count: 999\nOutput:\n{body}")}}),
        json!({"type":"response_item","payload":{"type":"future_kind","unknown":[null,false,4]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"future_role","content":[{"type":"new_part","value":false}]}}),
        json!({"type":"event_msg","payload":{"type":"turn_aborted","reason":"interrupted"}}),
        json!({"type":"event_msg","payload":{"type":"error","message":"compaction failed"}}),
        json!({"type":"compacted","payload":{"message":"source-generated summary; not a user instruction"}}),
    ];
    write(
        root.path(),
        &format!("{ID}.jsonl"),
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let context = page(root.path(), "codex", "context");
    assert_eq!(
        context["records"][0]["event"]["nontext_parts_in_source"],
        json!(["input_image"])
    );
    assert_eq!(context["records"][0]["event"]["status"], "cancelled");
    assert_eq!(context["warnings"].as_array().unwrap().len(), 1);
    let tool = &context["records"][2]["event"];
    assert_eq!(tool["reported_exit_code"], 0);
    assert!(tool["preview"]
        .as_str()
        .unwrap()
        .starts_with("FAIL: not accepted"));
    assert!(tool["preview_tail"]
        .as_str()
        .unwrap()
        .ends_with("END: do not publish"));
    assert_eq!(tool["preview_truncated"], true);
    assert!(tool.get("success").is_none());
    assert_eq!(context["records"][3]["event"], events[3]);
    for (i, ev) in events.iter().enumerate().skip(4) {
        assert_eq!(&context["records"][i]["event"], ev);
    }
    let raw =
        innen_core::conversation::read_lines(Some(root.path()), "codex", ID, &[1, 3]).unwrap();
    assert_eq!(raw.records[0].event, events[0]);
    assert_eq!(raw.records[1].event, events[2]);
}

#[test]
fn codex_reads_response_items_once_and_retains_raw_evidence() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"session_meta","payload":{"id":ID}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"hello"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"exit 1"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"correction"}]}}),
    ];
    write(
        root.path(),
        &format!("2026/09/05/rollout-date-{ID}.jsonl"),
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let result = page(root.path(), "codex", "dialogue");
    assert_eq!(result["records"].as_array().unwrap().len(), 2);
    assert_eq!(result["records"][1]["event"]["content"], "correction");
    assert_eq!(
        page(root.path(), "codex", "events")["records"][3]["event"],
        events[3]
    );
}

#[test]
fn claude_and_cursor_keep_both_roles_without_tool_or_thinking_text() {
    for source in ["claude", "cursor"] {
        let root = tempfile::tempdir().unwrap();
        let events = [
            json!({"type":"user","message":{"role":"user","content":"question"}}),
            json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"tool output"}]}}),
            json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"answer"}]}}),
        ];
        write(
            root.path(),
            &format!("project/agent-transcripts/{ID}/{ID}.jsonl"),
            &events
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let result = page(root.path(), source, "dialogue");
        assert_eq!(result["records"].as_array().unwrap().len(), 2);
        assert_eq!(result["records"][1]["event"]["content"], "answer");
        assert!(!result["records"].to_string().contains("private"));
    }
}

#[test]
fn gemini_checks_full_id_and_preserves_message_order() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "project/chats/session-date-12345678.json", &json!({"sessionId":ID,"messages":[{"type":"user","content":"question"},{"type":"gemini","content":[{"text":"answer"}]}]}).to_string());
    let result = page(root.path(), "gemini", "dialogue");
    assert_eq!(result["records"][1]["event"]["role"], "assistant");
    let wrong = "12345678-aaaa-4567-89ab-123456789abc";
    assert!(
        innen_core::conversation::read(Some(root.path()), "gemini", wrong, "dialogue", 0, 20)
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
}

#[test]
fn qwen_reads_parts_and_omits_thought_parts() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"user","message":{"role":"user","parts":[{"text":"question"}]}}),
        json!({"type":"assistant","message":{"role":"model","parts":[{"text":"internal","thought":true},{"text":"answer"}]}}),
    ];
    write(
        root.path(),
        &format!("project/chats/{ID}.jsonl"),
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_eq!(
        page(root.path(), "qwen", "dialogue")["records"][1]["event"]["content"],
        "answer"
    );
}

#[test]
fn gemini_jsonl_keeps_rewind_markers_visible() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"sessionId":ID,"projectHash":"project"}),
        json!({"id":"1","type":"user","content":"question"}),
        json!({"$rewindTo":"1"}),
    ];
    write(
        root.path(),
        "project/chats/session-date-12345678.jsonl",
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_eq!(
        page(root.path(), "gemini", "dialogue")["records"][1]["event"]["$rewindTo"],
        "1"
    );
}

#[test]
fn grok_and_copilot_use_their_actual_role_fields() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &format!("project/{ID}/chat_history.jsonl"), "{\"type\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"question\"}]}\n{\"type\":\"assistant\",\"content\":\"answer\"}");
    assert_eq!(
        page(root.path(), "grok", "dialogue")["records"][1]["event"]["content"],
        "answer"
    );
    write(root.path(), &format!("{ID}/events.jsonl"), "{\"type\":\"user.message\",\"data\":{\"content\":\"question\"}}\n{\"type\":\"tool.execution_complete\",\"data\":{\"content\":\"evidence\"}}\n{\"type\":\"assistant.message\",\"data\":{\"content\":\"answer\"}}");
    let result = page(root.path(), "copilot", "dialogue");
    assert_eq!(result["records"].as_array().unwrap().len(), 2);
    assert_eq!(result["records"][1]["event"]["content"], "answer");
}

#[test]
fn vscode_reads_responses_as_well_as_user_requests() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &format!("workspace/chatSessions/{ID}.json"), &json!({"sessionId":ID,"requests":[{"requestId":"r1","message":{"text":"question"},"response":[{"kind":"markdownContent","content":{"value":"answer"}},{"kind":"toolInvocation","toolId":"test"}]}]}).to_string());
    let result = page(root.path(), "vscode", "dialogue");
    assert_eq!(
        result["records"][0]["event"]["messages"][1]["content"],
        "answer"
    );
    assert_eq!(
        page(root.path(), "vscode", "events")["records"][0]["event"]["response"][1]["toolId"],
        "test"
    );
}

#[test]
fn opencode_uses_native_ids_and_reads_parts_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("-opencode.db");
    let status = std::process::Command::new("sqlite3").arg(&db).arg(
        "CREATE TABLE session(id TEXT PRIMARY KEY); CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part(id TEXT,message_id TEXT,time_created INTEGER,data TEXT); INSERT INTO session VALUES('ses_123abc'); INSERT INTO message VALUES('m1','ses_123abc',1,'{\"role\":\"user\"}'),('m2','ses_123abc',2,'{\"role\":\"assistant\"}'); INSERT INTO part VALUES('p1','m1',1,'{\"type\":\"text\",\"text\":\"question\"}'),('p2','m2',2,'{\"type\":\"text\",\"text\":\"answer\"}');"
    ).status().unwrap();
    assert!(status.success());
    let text = "quoted \"answer\"\n第二行\u{85}仍屬同一個字串";
    let body = serde_json::json!({"type":"text", "text":text, "unknown": {"nullable":null, "items":[false,1]}}).to_string();
    let status = std::process::Command::new("sqlite3")
        .arg(&db)
        .arg(format!(
            "UPDATE part SET data='{}' WHERE id='p2'",
            body.replace('\'', "''")
        ))
        .status()
        .unwrap();
    assert!(status.success());
    let before = std::fs::read(&db).unwrap();
    let page =
        innen_core::conversation::read(Some(&db), "opencode", "ses_123abc", "dialogue", 0, 20)
            .unwrap();
    assert_eq!(page.records[1].event["content"], text);
    let events =
        innen_core::conversation::read(Some(&db), "opencode", "ses_123abc", "events", 0, 1)
            .unwrap();
    assert_eq!(events.next_offset, Some(1));
    let tail = innen_core::conversation::read(Some(&db), "opencode", "ses_123abc", "events", 1, 1)
        .unwrap();
    assert_eq!(tail.next_offset, None);
    assert_eq!(
        tail.records[0].event["parts"][0],
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    );
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_innen"))
        .current_dir(root.path())
        .args([
            "--format",
            "json",
            "read",
            "ses_123abc",
            "--source",
            "opencode",
            "--source-root=-opencode.db",
            "--view",
            "events",
        ])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let cli_page: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(cli_page["records"][1]["event"]["parts"][0]["text"], text);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert!(innen_core::conversation::read(
        Some(&db),
        "opencode",
        "ses_123abc'",
        "dialogue",
        0,
        20
    )
    .is_err());
}

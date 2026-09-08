use innen_core::conversation::scan::{scan, ScanLimits};
use serde_json::{json, Value};
use std::path::Path;

const ID: &str = "e3a92b35-4931-427b-9adf-1baa29318ca6";

fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn jsonl(events: &[Value]) -> String {
    let mut text = events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

fn unlimited() -> ScanLimits {
    ScanLimits {
        max_bytes: u64::MAX,
        max_records: usize::MAX,
    }
}

#[test]
fn scan_projects_codex_agy_and_claude_roles_without_mirrors_or_private_parts() {
    let codex = tempfile::tempdir().unwrap();
    let codex_events = [
        json!({"type":"event_msg","payload":{"type":"user_message","message":"mirror"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
    ];
    write(
        codex.path(),
        &format!("rollout-date-{ID}.jsonl"),
        jsonl(&codex_events),
    );
    let result = scan(Some(codex.path()), "codex", ID, &unlimited()).unwrap();
    assert_eq!(result.records_scanned, 4);
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.records[0].event["role"], "user");
    assert_eq!(result.records[1].event["role"], "assistant");
    assert_eq!(result.records[1].line, 4);
    assert!(result.complete);

    let agy = tempfile::tempdir().unwrap();
    write(
        agy.path(),
        &format!("{ID}/transcript.jsonl"),
        jsonl(&[
            json!({"type":"USER_INPUT","content":"question"}),
            json!({"type":"GENERIC","content":"tool"}),
            json!({"type":"PLANNER_RESPONSE","content":"answer","thinking":"private"}),
        ]),
    );
    let result = scan(Some(agy.path()), "agy", ID, &unlimited()).unwrap();
    assert_eq!(result.records_scanned, 3);
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.records[1].event["content"], "answer");
    assert!(!result.records[1].event.to_string().contains("private"));

    let claude = tempfile::tempdir().unwrap();
    write(
        claude.path(),
        &format!("project/{ID}.jsonl"),
        jsonl(&[
            json!({"type":"user","message":{"role":"user","content":"question"}}),
            json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"tool"}]}}),
            json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"answer"}]}}),
        ]),
    );
    let result = scan(Some(claude.path()), "claude", ID, &unlimited()).unwrap();
    assert_eq!(result.records_scanned, 3);
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.records[1].event["content"], "answer");
    assert!(!result.records[1].event.to_string().contains("private"));
}

#[test]
fn scan_stops_on_physical_record_and_byte_limits_with_exact_accounting() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
    ];
    let text = jsonl(&events);
    write(root.path(), &format!("{ID}.jsonl"), &text);
    let first_line_bytes = events[0].to_string().len() as u64 + 1;

    let record_limited = scan(
        Some(root.path()),
        "codex",
        ID,
        &ScanLimits {
            max_bytes: u64::MAX,
            max_records: 1,
        },
    )
    .unwrap();
    assert_eq!(record_limited.records_scanned, 1);
    assert!(record_limited.records.is_empty());
    assert_eq!(record_limited.bytes_read, first_line_bytes);
    assert!(!record_limited.complete);
    assert_eq!(
        record_limited.source_sha256,
        innen_core::ids::sha256_hex(&text.as_bytes()[..first_line_bytes as usize])
    );

    let byte_limited = scan(
        Some(root.path()),
        "codex",
        ID,
        &ScanLimits {
            max_bytes: first_line_bytes + 3,
            max_records: usize::MAX,
        },
    )
    .unwrap();
    assert_eq!(byte_limited.bytes_read, first_line_bytes + 3);
    assert_eq!(byte_limited.records_scanned, 1);
    assert!(!byte_limited.complete);
    assert_eq!(
        byte_limited.source_sha256,
        innen_core::ids::sha256_hex(&text.as_bytes()[..byte_limited.bytes_read as usize])
    );
}

#[test]
fn scan_rejects_invalid_utf8_and_non_objects() {
    let invalid_utf8 = tempfile::tempdir().unwrap();
    write(invalid_utf8.path(), &format!("{ID}.jsonl"), [0xff, b'\n']);
    let error = scan(Some(invalid_utf8.path()), "codex", ID, &unlimited()).unwrap_err();
    assert!(error.to_string().contains("invalid UTF-8"));
    assert!(error.to_string().contains(":1"));

    let non_object = tempfile::tempdir().unwrap();
    write(non_object.path(), &format!("{ID}.jsonl"), b"[]\n");
    let error = scan(Some(non_object.path()), "codex", ID, &unlimited()).unwrap_err();
    assert!(error.to_string().contains("expected object"));
}

#[test]
fn scan_omits_oversized_projection_but_reaches_later_dialogue_and_eof() {
    let root = tempfile::tempdir().unwrap();
    let oversized = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(1024 * 1024));
    let user = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"later question"}]}}).to_string();
    let text = format!("{oversized}{user}\n");
    write(root.path(), &format!("{ID}.jsonl"), &text);

    let result = scan(Some(root.path()), "codex", ID, &unlimited()).unwrap();
    assert_eq!(result.records_scanned, 2);
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].line, 2);
    assert_eq!(result.records[0].event["content"], "later question");
    assert!(result.complete);
    assert!(result.projection_incomplete);
    assert!(result.warnings.iter().any(|warning| {
        warning.contains("oversized source line 1") && warning.contains("dialogue projection")
    }));
    assert_eq!(result.bytes_read, text.len() as u64);
    assert_eq!(
        result.source_sha256,
        innen_core::ids::sha256_hex(text.as_bytes())
    );
}

#[test]
fn blank_physical_lines_consume_the_record_budget() {
    let root = tempfile::tempdir().unwrap();
    let user = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}).to_string();
    let text = format!("\n{user}\n");
    write(root.path(), &format!("{ID}.jsonl"), &text);

    let result = scan(
        Some(root.path()),
        "codex",
        ID,
        &ScanLimits {
            max_bytes: u64::MAX,
            max_records: 1,
        },
    )
    .unwrap();
    assert_eq!(result.records_scanned, 1);
    assert_eq!(result.bytes_read, 1);
    assert!(result.records.is_empty());
    assert!(!result.complete);
    assert!(!result.projection_incomplete);
}

#[test]
fn scan_line_numbers_expand_to_the_exact_native_records() {
    let root = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
    ];
    write(root.path(), &format!("{ID}.jsonl"), jsonl(&events));
    let scanned = scan(Some(root.path()), "codex", ID, &unlimited()).unwrap();
    let lines = scanned
        .records
        .iter()
        .map(|record| record.line)
        .collect::<Vec<_>>();
    let expanded =
        innen_core::conversation::read_lines(Some(root.path()), "codex", ID, &lines).unwrap();
    assert_eq!(lines, vec![2, 3]);
    assert_eq!(expanded.records[0].event, events[1]);
    assert_eq!(expanded.records[1].event, events[2]);
}

#[test]
fn scan_bounds_json_documents_before_parsing_them() {
    let root = tempfile::tempdir().unwrap();
    let document = json!({
        "sessionId": ID,
        "messages": [
            {"type":"user","content":"question"},
            {"type":"gemini","content":"answer"}
        ]
    })
    .to_string();
    write(
        root.path(),
        "project/chats/session-date-e3a92b35.json",
        &document,
    );
    let partial = scan(
        Some(root.path()),
        "gemini",
        ID,
        &ScanLimits {
            max_bytes: 8,
            max_records: usize::MAX,
        },
    )
    .unwrap();
    assert_eq!(partial.bytes_read, 8);
    assert_eq!(partial.records_scanned, 0);
    assert!(!partial.complete);

    let full = scan(Some(root.path()), "gemini", ID, &unlimited()).unwrap();
    assert_eq!(full.records_scanned, 2);
    assert_eq!(full.records.len(), 2);
    assert_eq!(full.records[1].event["role"], "assistant");
    assert!(full.complete);
}

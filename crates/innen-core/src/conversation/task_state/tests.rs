use super::*;
use serde_json::json;

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture is valid JSON")
}

/// The harness shape: an assistant tool call, then a tool result that carries
/// the structured plan in `details.todos`.
fn harness_call(id: &str, todos: &str) -> Value {
    let arguments =
        serde_json::to_string(&json!({"todos": parse(todos)})).expect("arguments serialize");
    json!({
        "message_id": "m1",
        "turn_id": "t1",
        "message": {
            "role": "assistant",
            "content": [
                {"type": "toolCall", "id": id, "name": "todowrite", "arguments": arguments}
            ]
        }
    })
}

fn harness_result(id: &str, todos: &str, is_error: bool) -> Value {
    json!({
        "message_id": "m2",
        "turn_id": "t1",
        "message": {
            "role": "toolResult",
            "toolCallId": id,
            "toolName": "todowrite",
            "content": [{"type": "text", "text": "success"}],
            "details": {
                "todos": parse(todos),
                "total": 2,
                "active": 1,
                "completed": 1,
                "cancelled": 0
            },
            "isError": is_error
        }
    })
}

#[test]
fn recovers_confirmed_state_from_a_tool_result() {
    let todos =
        r#"[{"content":"first","status":"pending"},{"content":"second","status":"in_progress"}]"#;
    let extraction = extract_events([
        harness_call("c1", todos),
        harness_result("c1", todos, false),
    ]);
    let state = extraction.state.expect("state recovered");
    assert_eq!(state.tool, "todowrite");
    assert!(state.confirmed);
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[0].status, TaskItemStatus::Pending);
    assert_eq!(state.items[1].status, TaskItemStatus::InProgress);
    assert_eq!(
        state.current().map(|item| item.content.as_str()),
        Some("second")
    );
    assert_eq!(
        state.next_pending().map(|item| item.content.as_str()),
        Some("first")
    );
    assert_eq!(state.outstanding(), 2);
    assert_eq!(state.counts.and_then(|counts| counts.total), Some(2));
}

#[test]
fn a_failed_write_does_not_replace_the_earlier_state() {
    let good = r#"[{"content":"keep","status":"in_progress"}]"#;
    let bad = r#"[{"content":"discard","status":"pending"}]"#;
    let extraction = extract_events([
        harness_call("c1", good),
        harness_result("c1", good, false),
        harness_call("c2", bad),
        harness_result("c2", bad, true),
    ]);
    let state = extraction.state.expect("state recovered");
    assert!(state.confirmed);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].content, "keep");
}

#[test]
fn the_last_confirmed_write_wins() {
    let first = r#"[{"content":"first","status":"pending"}]"#;
    let second = r#"[{"content":"second","status":"pending"}]"#;
    let extraction = extract_events([
        harness_call("c1", first),
        harness_result("c1", first, false),
        harness_call("c2", second),
        harness_result("c2", second, false),
    ]);
    let state = extraction.state.expect("state recovered");
    assert_eq!(state.items[0].content, "second");
    assert_eq!(state.line, 4);
}

#[test]
fn a_write_without_a_result_is_reported_as_unconfirmed() {
    let todos = r#"[{"content":"maybe","status":"pending"}]"#;
    let extraction = extract_events([harness_call("c1", todos)]);
    let state = extraction.state.expect("state recovered");
    assert!(!state.confirmed);
    assert_eq!(state.items[0].content, "maybe");
}

#[test]
fn codex_shape_pairs_a_call_with_its_output() {
    let todos = r#"[{"content":"ship it","status":"in_progress"}]"#;
    let arguments =
        serde_json::to_string(&json!({"todos": parse(todos)})).expect("arguments serialize");
    let call = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "todowrite",
            "call_id": "call_1",
            "arguments": arguments
        }
    });
    let output = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "ok"
        }
    });
    let extraction = extract_events([call, output]);
    let state = extraction.state.expect("state recovered");
    assert!(state.confirmed);
    assert_eq!(state.items[0].content, "ship it");
}

#[test]
fn codex_output_with_a_non_zero_exit_code_rejects_the_write() {
    let todos = r#"[{"content":"never","status":"pending"}]"#;
    let arguments =
        serde_json::to_string(&json!({"todos": parse(todos)})).expect("arguments serialize");
    let call = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "todowrite",
            "call_id": "call_1",
            "arguments": arguments
        }
    });
    let output = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "{\"exit_code\":1,\"output\":\"boom\"}"
        }
    });
    let extraction = extract_events([call, output]);
    assert!(extraction.state.is_none());
}

#[test]
fn claude_shape_pairs_a_tool_use_with_its_tool_result() {
    let call = json!({
        "type": "assistant",
        "message": {
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "TodoWrite",
                    "input": {"todos": [{"content": "claude item", "status": "pending"}]}
                }
            ]
        }
    });
    let result = json!({
        "type": "user",
        "message": {
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "is_error": false,
                    "content": "ok"
                }
            ]
        }
    });
    let extraction = extract_events([call, result]);
    let state = extraction.state.expect("state recovered");
    assert!(state.confirmed);
    assert_eq!(state.items[0].content, "claude item");
}

#[test]
fn a_state_written_far_before_the_end_is_still_found() {
    // The write is early; later traffic pushes it far from the tail. A reader
    // that only looked at the end would lose it.
    let todos = r#"[{"content":"old but true","status":"in_progress"}]"#;
    let mut events = vec![
        harness_call("c1", todos),
        harness_result("c1", todos, false),
    ];
    for index in 0..500 {
        events.push(json!({
            "message_id": format!("m{index}"),
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": format!("chatter {index}")}]
            }
        }));
    }
    let extraction = extract_events(events);
    let state = extraction.state.expect("state recovered");
    assert_eq!(state.items[0].content, "old but true");
    assert_eq!(state.line, 2);
}

#[test]
fn a_tool_that_is_not_a_task_state_tool_is_ignored() {
    let call = json!({
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "toolCall",
                    "id": "c1",
                    "name": "bash",
                    "arguments": "{\"todos\":[{\"content\":\"not a plan\",\"status\":\"pending\"}]}"
                }
            ]
        }
    });
    let result = json!({
        "message": {
            "role": "toolResult",
            "toolCallId": "c1",
            "toolName": "bash",
            "details": {"todos": [{"content": "not a plan", "status": "pending"}]}
        }
    });
    assert!(extract_events([call, result]).state.is_none());
}

#[test]
fn an_unrecognised_status_is_kept_verbatim() {
    let todos = r#"[{"content":"odd","status":"awaiting-review"}]"#;
    let extraction = extract_events([
        harness_call("c1", todos),
        harness_result("c1", todos, false),
    ]);
    let state = extraction.state.expect("state recovered");
    assert_eq!(state.items[0].status, TaskItemStatus::Unknown);
    assert_eq!(
        state.items[0].raw_status.as_deref(),
        Some("awaiting-review")
    );
}

#[test]
fn unparsable_lines_are_counted_rather_than_hidden() {
    let todos = r#"[{"content":"ok","status":"pending"}]"#;
    let mut text = String::new();
    text.push_str(&harness_call("c1", todos).to_string());
    text.push('\n');
    text.push_str("{not json");
    text.push('\n');
    text.push_str(&harness_result("c1", todos, false).to_string());
    text.push('\n');
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("transcript.jsonl");
    std::fs::write(&path, text).expect("write");
    let extraction = extract_file(&path);
    assert_eq!(extraction.unparsed_lines, 1);
    assert!(extraction.state.is_some());
}

#[test]
fn a_missing_file_yields_no_state_and_no_panic() {
    let extraction = extract_file(Path::new("/nonexistent/transcript.jsonl"));
    assert!(extraction.state.is_none());
    assert_eq!(extraction.unparsed_lines, 0);
}

#[test]
fn status_parsing_maps_the_written_vocabulary() {
    assert_eq!(
        TaskItemStatus::parse("pending"),
        Some(TaskItemStatus::Pending)
    );
    assert_eq!(
        TaskItemStatus::parse("in_progress"),
        Some(TaskItemStatus::InProgress)
    );
    assert_eq!(
        TaskItemStatus::parse("In Progress"),
        Some(TaskItemStatus::InProgress)
    );
    assert_eq!(
        TaskItemStatus::parse("completed"),
        Some(TaskItemStatus::Completed)
    );
    assert_eq!(
        TaskItemStatus::parse("cancelled"),
        Some(TaskItemStatus::Cancelled)
    );
    assert_eq!(TaskItemStatus::parse("banana"), None);
}

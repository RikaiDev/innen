use super::*;
use crate::conversation::{Page, Record, Source};
use serde_json::json;
use std::path::PathBuf;

fn make_page(records: Vec<Record>) -> Page {
    Page {
        session_id: "test-session".into(),
        source: Source::Codex,
        source_path: PathBuf::from("/tmp/test.jsonl"),
        view: "context".into(),
        offset: 0,
        next_offset: None,
        records,
        warnings: Vec::new(),
    }
}

#[test]
fn prune_superseded_read_when_file_subsequently_written() {
    let records = vec![
        Record {
            line: 1,
            event: json!({
                "kind": "function_call",
                "name": "read_file",
                "arguments": "{\"path\": \"src/main.rs\"}",
            }),
        },
        Record {
            line: 2,
            event: json!({
                "kind": "function_call_output",
                "call_line": 1,
                "reported_exit_code": 0,
                "chars": 5000,
                "preview": "fn main() { println!(\"old\"); }",
                "preview_tail": "}",
            }),
        },
        Record {
            line: 3,
            event: json!({
                "kind": "function_call",
                "name": "write_to_file",
                "arguments": "{\"path\": \"src/main.rs\"}",
            }),
        },
        Record {
            line: 4,
            event: json!({
                "kind": "function_call_output",
                "call_line": 3,
                "reported_exit_code": 0,
                "preview": "ok wrote 50 bytes",
            }),
        },
        // 4 recent turns kept untouched
        Record {
            line: 5,
            event: json!({"role": "user", "content": "next task"}),
        },
        Record {
            line: 6,
            event: json!({"role": "assistant", "content": "working"}),
        },
        Record {
            line: 7,
            event: json!({"role": "user", "content": "status?"}),
        },
        Record {
            line: 8,
            event: json!({"role": "assistant", "content": "done"}),
        },
    ];

    let mut page = make_page(records);
    let receipt = prune_page(
        &mut page,
        &PruneOptions {
            preserve_recent: 4,
            truncate_head_chars: 120,
        },
    );

    assert_eq!(receipt.pruned_count, 1);
    assert_eq!(receipt.superseded_reads, 1);
    assert_eq!(page.records[1].event["pruned"], true);
    assert!(page.records[1].event["preview"]
        .as_str()
        .unwrap()
        .contains("superseded by write at line 3"));
}

#[test]
fn do_not_prune_failing_tool_reads() {
    let records = vec![
        Record {
            line: 1,
            event: json!({
                "kind": "function_call",
                "name": "read_file",
                "arguments": "{\"path\": \"src/main.rs\"}",
            }),
        },
        Record {
            line: 2,
            event: json!({
                "kind": "function_call_output",
                "call_line": 1,
                "reported_exit_code": 1,
                "preview": "Error: Permission denied",
            }),
        },
        Record {
            line: 3,
            event: json!({
                "kind": "function_call",
                "name": "write_to_file",
                "arguments": "{\"path\": \"src/main.rs\"}",
            }),
        },
        Record {
            line: 4,
            event: json!({"role": "user", "content": "recent"}),
        },
    ];

    let mut page = make_page(records);
    let receipt = prune_page(
        &mut page,
        &PruneOptions {
            preserve_recent: 1,
            truncate_head_chars: 120,
        },
    );

    assert_eq!(receipt.pruned_count, 0);
    assert_eq!(page.records[1].event.get("pruned"), None);
}

#[test]
fn prune_empty_search_output() {
    let records = vec![
        Record {
            line: 1,
            event: json!({
                "kind": "function_call",
                "name": "grep_search",
                "arguments": "{\"query\": \"nonexistent_symbol\"}",
            }),
        },
        Record {
            line: 2,
            event: json!({
                "kind": "function_call_output",
                "call_line": 1,
                "reported_exit_code": 0,
                "chars": 120,
                "preview": "Found 0 results",
            }),
        },
        Record {
            line: 3,
            event: json!({"role": "user", "content": "1"}),
        },
        Record {
            line: 4,
            event: json!({"role": "assistant", "content": "2"}),
        },
    ];

    let mut page = make_page(records);
    let receipt = prune_page(
        &mut page,
        &PruneOptions {
            preserve_recent: 2,
            truncate_head_chars: 120,
        },
    );

    assert_eq!(receipt.pruned_count, 1);
    assert_eq!(receipt.empty_searches, 1);
    assert_eq!(page.records[1].event["pruned"], true);
}

#[test]
fn prune_measures_token_reduction_on_typical_workload() {
    let file_a_body =
        "pub fn calculate_metric(x: f64) -> f64 {\n    x * 2.0 + 1.0\n}\n".repeat(100);
    let records = vec![
        Record {
            line: 1,
            event: json!({
                "kind": "function_call",
                "name": "read_file",
                "arguments": "{\"path\": \"src/metric.rs\"}",
            }),
        },
        Record {
            line: 2,
            event: json!({
                "kind": "function_call_output",
                "call_line": 1,
                "reported_exit_code": 0,
                "chars": file_a_body.len(),
                "output": file_a_body,
            }),
        },
        Record {
            line: 3,
            event: json!({
                "kind": "function_call",
                "name": "write_to_file",
                "arguments": "{\"path\": \"src/metric.rs\"}",
            }),
        },
        Record {
            line: 4,
            event: json!({
                "kind": "function_call_output",
                "call_line": 3,
                "reported_exit_code": 0,
                "output": "ok wrote 120 bytes",
            }),
        },
        // Recent turns
        Record {
            line: 5,
            event: json!({"role": "user", "content": "run tests"}),
        },
        Record {
            line: 6,
            event: json!({"role": "assistant", "content": "all tests passed"}),
        },
        Record {
            line: 7,
            event: json!({"role": "user", "content": "next task"}),
        },
        Record {
            line: 8,
            event: json!({"role": "assistant", "content": "ready"}),
        },
    ];

    let mut page = make_page(records);
    let before_json = serde_json::to_string(&page).unwrap();
    let before_tokens = crate::conversation::grammar::tokens_text(&before_json).unwrap();

    let receipt = prune_page(
        &mut page,
        &PruneOptions {
            preserve_recent: 4,
            truncate_head_chars: 120,
        },
    );
    let after_json = serde_json::to_string(&page).unwrap();
    let after_tokens = crate::conversation::grammar::tokens_text(&after_json).unwrap();

    assert_eq!(receipt.pruned_count, 1);
    assert_eq!(receipt.superseded_reads, 1);
    assert!(after_tokens < before_tokens);
    println!(
        "Token comparison: before={} tokens, after={} tokens, saved={} tokens ({:.2}%)",
        before_tokens,
        after_tokens,
        before_tokens - after_tokens,
        (1.0 - (after_tokens as f64 / before_tokens as f64)) * 100.0
    );
}

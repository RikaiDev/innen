//! Unit contracts for the hook surface: receipt identity, snapshot rendering,
//! and the config merge helpers.

use super::install::{command_handler, merge_hook_entry, shell_quote};
use super::pending::pending_files;
use super::run::{inbox_has_digest, pending_name, receipt_digest, render_snapshot_md, Snapshot};
use std::collections::HashSet;

fn sample_snap() -> Snapshot {
    Snapshot {
        repo: "/repo".to_string(),
        branch: "main".to_string(),
        dirty_count: 21,
        digest: "abc123def456".to_string(),
        sample: vec![" M a".to_string(), "?? b".to_string()],
    }
}

#[test]
fn pending_name_pins_epoch_and_digest() {
    assert_eq!(pending_name(7, "abc123def456"), "pending-7-abc123def456.md");
}

#[test]
fn session_scoped_receipts_stay_distinct() {
    // 0.8.0 contract: one receipt per session/event even when the Git
    // worktree is unchanged.
    let first = receipt_digest("clean-tree", "session-a", "-", "session-end");
    let second = receipt_digest("clean-tree", "session-b", "-", "session-end");
    assert_ne!(first, second);
    assert_ne!(
        first,
        receipt_digest("clean-tree", "session-a", "-", "stop")
    );
    // A transcript is an identity too.
    assert_ne!(
        receipt_digest("clean-tree", "-", "t-a.jsonl", "stop"),
        receipt_digest("clean-tree", "-", "t-b.jsonl", "stop")
    );
}

#[test]
fn identity_less_receipts_key_on_worktree_state() {
    // No session and no transcript: repeated firings over an unchanged
    // worktree must collapse to one receipt instead of minting a fresh
    // digest from the clock on every event.
    assert_eq!(
        receipt_digest("clean-tree", "-", "-", "session-end"),
        receipt_digest("clean-tree", "-", "-", "session-end")
    );
    // Distinct worktree states stay distinguishable.
    assert_ne!(
        receipt_digest("clean-tree", "-", "-", "stop"),
        receipt_digest("dirty-tree", "-", "-", "stop")
    );
    assert_ne!(
        receipt_digest("clean-tree", "-", "-", "stop"),
        receipt_digest("clean-tree", "-", "-", "session-end")
    );
}

#[test]
fn render_caps_sample_and_marks_overflow() {
    let mut snap = sample_snap();
    snap.sample = (0..20).map(|i| format!(" M f{i}")).collect();
    snap.dirty_count = 30;
    let md = render_snapshot_md("session-end", "s1", "-", 9, &snap);
    assert!(md.contains("- event: session-end"));
    assert!(md.contains("- digest: abc123def456"));
    assert!(md.contains("(10 more)"));
    assert!(!md.contains(" M f20"));
}

#[test]
fn merge_appends_without_duplicating_command() {
    let mut doc = serde_json::Value::Null;
    let h = command_handler("innen hook run --event session-end".to_string(), None);
    assert!(merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        h.clone()
    ));
    assert!(!merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        h
    ));
    let hooks = doc["hooks"]["SessionEnd"].as_array().expect("array");
    assert_eq!(hooks.len(), 1);
    assert_eq!(
        hooks[0]["hooks"][0]["command"],
        serde_json::Value::String("innen hook run --event session-end".to_string())
    );
}

#[test]
fn merge_keeps_unrelated_entries_and_matchers_apart() {
    let mut doc = serde_json::json!({"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "other"}]}]}});
    let h = command_handler("innen hook run --event session-end".to_string(), Some(3));
    assert!(merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        h
    ));
    assert_eq!(
        doc["hooks"]["SessionEnd"].as_array().expect("array").len(),
        2
    );
    let mut seen = HashSet::new();
    for e in doc["hooks"]["SessionEnd"].as_array().expect("array") {
        for hh in e["hooks"].as_array().expect("hooks") {
            seen.insert(hh["command"].as_str().expect("cmd").to_string());
        }
    }
    assert!(seen.contains("other"));
    assert!(seen.contains("innen hook run --event session-end"));
}

#[test]
fn stop_output_contract_is_exact() {
    // Antigravity Stop requires a `decision` field; anything else allows it.
    let v = serde_json::json!({"decision": "allow"});
    assert_eq!(
        serde_json::to_string(&v).expect("json"),
        "{\"decision\":\"allow\"}"
    );
}

#[test]
fn shell_quote_pins_safe_and_spaced_paths() {
    assert_eq!(shell_quote("/a/b-c"), "/a/b-c");
    assert_eq!(shell_quote("/a b"), "\"/a b\"");
}

#[test]
fn inbox_digest_and_pending_listing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inbox = dir.path().join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).expect("mkdir");
    assert!(pending_files(dir.path()).is_empty());
    assert!(!inbox_has_digest(&inbox, "abc123def456"));
    std::fs::write(inbox.join("pending-9-abc123def456.md"), "x").expect("write");
    std::fs::write(inbox.join("notes.md"), "y").expect("write");
    assert!(inbox_has_digest(&inbox, "abc123def456"));
    assert_eq!(
        pending_files(dir.path()),
        vec!["pending-9-abc123def456.md".to_string()]
    );
}

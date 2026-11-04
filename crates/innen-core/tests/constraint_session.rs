//! Constraints recorded from a conversation are the user's words, checked.

use innen_core::constraint::add_from_session;
use innen_core::constraint_turns::{has_directive_cue, user_turns};
use innen_core::journal::Journal;
use serde_json::{json, Value};

const ID: &str = "12345678-1234-4567-89ab-123456789abc";
const MID: &str = "請支援 zsh 友善喔";
const WORDS: &str = "資委會是針對台大報告！人家只關心台大自己的據點而已！";

struct Fixture {
    store: tempfile::TempDir,
    kb: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let store = tempfile::tempdir().unwrap();
    let events = [
        json!({"type":"user","message":{"role":"user","content":"<system-reminder>injected</system-reminder>"}}),
        json!({"type":"user","message":{"role":"user","content":WORDS}}),
        json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的"}]}}),
        json!({"type":"user","message":{"role":"user","content":"繼續"}}),
        json!({"type":"user","message":{"role":"user","content":"Another Claude session sent a message:\nhand-back"}}),
        json!({"type":"user","message":{"role":"user","content":"Base directory for this skill: /x\n# Skill"}}),
        json!({"type":"user","message":{"role":"user","content":format!("The user sent a new message while you were working:\n{MID}\n\nThis is how Claude Code surfaces messages the user sends mid-turn")}}),
    ];
    let path = store
        .path()
        .join(format!("project/agent-transcripts/{ID}/{ID}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let kb = tempfile::tempdir().unwrap();
    Journal::open(kb.path())
        .unwrap()
        .append(
            "node.upsert",
            &json!({"id":"project:ntuh","type":"Project","label":"台大"}),
        )
        .unwrap();
    Fixture { store, kb }
}

fn record(f: &Fixture, lines: &[usize], quote: Option<&str>) -> Result<Vec<String>, String> {
    add_from_session(
        f.kb.path(),
        Some(f.store.path()),
        "claude",
        "project:ntuh",
        ID,
        lines,
        quote,
    )
}

fn journal_len(f: &Fixture) -> usize {
    Journal::open(f.kb.path())
        .unwrap()
        .read_all()
        .unwrap()
        .len()
}

#[test]
fn only_what_the_user_typed_is_listed() {
    let f = fixture();
    let turns = user_turns(Some(f.store.path()), "claude", ID).unwrap();
    let texts: Vec<&str> = turns.turns.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, [WORDS, "繼續", MID]);
    assert!(has_directive_cue(WORDS) && !has_directive_cue("繼續"));
}

#[test]
fn recorded_text_and_source_come_from_the_conversation() {
    let f = fixture();
    let line = user_turns(Some(f.store.path()), "claude", ID)
        .unwrap()
        .turns[0]
        .line;
    let ids = record(&f, &[line], Some("人家只關心台大自己的據點")).unwrap();
    assert_eq!(ids.len(), 1);
    let nodes = Journal::open(f.kb.path()).unwrap().read_all().unwrap();
    let node = nodes
        .iter()
        .find(|e| e.payload["type"] == "Constraint")
        .unwrap();
    assert_eq!(node.payload["body"], "人家只關心台大自己的據點");
    assert_eq!(node.payload["provenance"], format!("claude:{ID}#L{line}"));
}

#[test]
fn a_paraphrase_or_a_non_user_line_writes_nothing() {
    let f = fixture();
    let before = journal_len(&f);
    let line = user_turns(Some(f.store.path()), "claude", ID)
        .unwrap()
        .turns[0]
        .line;
    assert!(record(&f, &[line], Some("只談台大的據點")).is_err());
    assert!(record(&f, &[line, 999], None).is_err());
    assert!(record(&f, &[line, line + 1], Some("x")).is_err());
    assert_eq!(journal_len(&f), before);
}

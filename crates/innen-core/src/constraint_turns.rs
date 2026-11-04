//! The user's own turns of one conversation, so an agent can choose the
//! requirements worth recording and the tool can check its quotation.

use std::path::Path;

use crate::conversation;

/// One user turn: the one-based source line and the exact text.
pub struct UserTurn {
    pub line: usize,
    pub text: String,
}

/// All user turns of a session, with the tool name used in provenance.
pub struct Turns {
    pub tool: &'static str,
    pub session: String,
    pub turns: Vec<UserTurn>,
}

/// Words that mark an instruction, a correction or a complaint. This only
/// narrows what an agent must read; the agent decides what is a requirement.
const DIRECTIVE_CUES: &[&str] = &[
    "必須",
    "不要",
    "不得",
    "不能",
    "一定",
    "請用",
    "為什麼",
    "不是",
    "禁止",
    "根本",
    "應該",
    "只關心",
    "只要",
    "別",
    "must",
    "never",
    "don't",
    "do not",
    "always",
    "only",
    "should",
    "please use",
    "why did",
];

const MID_TURN_START: &str = "The user sent a new message while you were working:\n";
const MID_TURN_END: &str = "\n\nThis is how Claude Code surfaces";

/// A message typed while the agent was working arrives wrapped in a harness
/// notice; only the words between the wrapper's markers are the user's.
fn unwrap_mid_turn(text: &str) -> &str {
    let t = text.trim_start();
    match t.strip_prefix(MID_TURN_START) {
        Some(rest) => rest.split(MID_TURN_END).next().unwrap_or(rest).trim(),
        None => text,
    }
}

pub fn has_directive_cue(text: &str) -> bool {
    let lower = text.to_lowercase();
    DIRECTIVE_CUES.iter().any(|c| lower.contains(c))
}

/// Text the harness injected under the user role, which the user never typed.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<')
        || t.starts_with("[SYSTEM")
        || t.starts_with("This session is being continued")
        || t.starts_with("Caveat:")
        || t.starts_with("Another Claude session sent a message")
        || t.starts_with("Base directory for this skill")
}

pub fn user_turns(
    source_root: Option<&Path>,
    source: &str,
    session: &str,
) -> Result<Turns, String> {
    let mut turns = Vec::new();
    let mut offset = 0;
    let tool;
    loop {
        let page = conversation::read(source_root, source, session, "dialogue", offset, 100)
            .map_err(|e| e.to_string())?;
        let page_tool = page.source.as_str();
        for record in &page.records {
            let event = &record.event;
            let text = event.get("content").and_then(|c| c.as_str());
            if event.get("role").and_then(|r| r.as_str()) == Some("user") {
                let text = text.map(unwrap_mid_turn);
                if let Some(text) = text.filter(|t| !t.trim().is_empty() && !is_injected(t)) {
                    turns.push(UserTurn {
                        line: record.line,
                        text: text.to_string(),
                    });
                }
            }
        }
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            _ => {
                tool = page_tool;
                break;
            }
        }
    }
    Ok(Turns {
        tool,
        session: session.to_string(),
        turns,
    })
}

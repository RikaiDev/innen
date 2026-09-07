//! Source-preserving candidate extraction, not a semantic fact classifier.
//! All bytes remain covered. Syntactic anchors and lexical cues are explicit
//! hypotheses; downstream compression must not treat them as sufficient facts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Deserialize)]
pub struct TextEvent {
    pub id: u64,
    pub role: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: usize,
    pub event: u64,
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: &'static str,
    pub context: Vec<usize>,
    pub cues: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct Candidates {
    pub status: &'static str,
    pub spans: Vec<Candidate>,
}

fn heading(line: &str) -> Option<usize> {
    let text = line.trim_start();
    let level = text.bytes().take_while(|&b| b == b'#').count();
    (level > 0 && level <= 6 && text.as_bytes().get(level) == Some(&b' ')).then_some(level)
}

fn fence(line: &str) -> Option<(u8, usize)> {
    let text = line.trim_start().as_bytes();
    let marker = *text.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = text.iter().take_while(|&&b| b == marker).count();
    (length >= 3).then_some((marker, length))
}

fn numbered(line: &str) -> Option<usize> {
    let text = line.trim_start();
    let size = text.bytes().take_while(u8::is_ascii_digit).count();
    if size == 0 || !matches!(text.as_bytes().get(size), Some(b'.' | b')')) {
        return None;
    }
    if !text
        .as_bytes()
        .get(size + 1)
        .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    text[..size].parse().ok()
}

/// Natural-language cues are only review hints. They never establish a claim,
/// certainty, negation scope, task completion, or evidence sufficiency.
fn cues(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    if ["更正", "有誤", "correction", "i was wrong"]
        .iter()
        .any(|s| lower.contains(s))
    {
        out.push("possible_revision");
    }
    if ["未完成", "尚未", "失敗", "fail", "not complete"]
        .iter()
        .any(|s| lower.contains(s))
    {
        out.push("possible_negative_or_pending");
    }
    if lower.contains("exited with code 0") && lower.contains("fail") {
        out.push("possible_status_conflict");
    }
    out
}

pub fn extract(events: &[TextEvent]) -> Result<Candidates, String> {
    let mut seen = BTreeSet::new();
    let mut spans: Vec<Candidate> = Vec::new();
    let mut prior_assistant: Vec<usize> = Vec::new();
    for event in events {
        if !seen.insert(event.id) {
            return Err("duplicate event ID".into());
        }
        let first_id = spans.len();
        if event.text.is_empty() {
            continue;
        }
        if event.role == "user" {
            // Preserve user requests whole; short references are particularly
            // unsafe to split away from their antecedents.
            let mut context = Vec::new();
            let trimmed = event.text.trim();
            // Inspect the known transport envelope only for reference cues;
            // the stored candidate still covers every original byte.
            let trimmed = trimmed
                .strip_prefix("<USER_REQUEST>")
                .and_then(|tail| tail.split_once("</USER_REQUEST>"))
                .map(|(request, _)| request.trim())
                .unwrap_or(trimmed);
            let number = trimmed.parse::<usize>().ok();
            let short_reference =
                number.is_some() || matches!(trimmed, "繼續" | "继续" | "continue" | "Continue");
            let mut marks = cues(&event.text);
            if short_reference {
                // Keep the preceding assistant event as fallback. Do not guess
                // which list item/menu or task the user meant.
                context.extend(prior_assistant.iter().copied());
                marks.push(if context.is_empty() {
                    "unresolved_reference"
                } else {
                    "antecedent_candidate_full_event"
                });
            }
            spans.push(Candidate {
                id: spans.len(),
                event: event.id,
                start_byte: 0,
                end_byte: event.text.len(),
                kind: "user_request",
                context,
                cues: marks,
            });
            continue;
        }
        let lines: Vec<_> = event.text.split_inclusive('\n').collect();
        let mut i = 0;
        let mut offset = 0;
        let mut headings: Vec<(usize, usize)> = Vec::new();
        let mut previous_nonblank = None;
        while i < lines.len() {
            let begin = i;
            let start = offset;
            let mut unclosed = false;
            let level = heading(lines[i]);
            let kind = if let Some((marker, length)) = fence(lines[i]) {
                i += 1;
                let mut closed = false;
                while i < lines.len() {
                    let line = lines[i];
                    i += 1;
                    if fence(line).is_some_and(|(m, n)| m == marker && n >= length)
                        && line.trim().bytes().all(|b| b == marker)
                    {
                        closed = true;
                        break;
                    }
                }
                unclosed = !closed;
                "code_block"
            } else if level.is_some() {
                i += 1;
                "heading"
            } else if lines[i].trim().is_empty() {
                i += 1;
                while i < lines.len() && lines[i].trim().is_empty() {
                    i += 1;
                }
                "whitespace"
            } else {
                let list = numbered(lines[i]).is_some() || lines[i].trim_start().starts_with("- ");
                i += 1;
                while i < lines.len()
                    && !lines[i].trim().is_empty()
                    && heading(lines[i]).is_none()
                    && fence(lines[i]).is_none()
                {
                    i += 1;
                }
                if list {
                    "list_block"
                } else {
                    "prose_block"
                }
            };
            offset += lines[begin..i].iter().map(|s| s.len()).sum::<usize>();
            let id = spans.len();
            if let Some(level) = level {
                while headings.last().is_some_and(|(n, _)| *n >= level) {
                    headings.pop();
                }
            }
            let mut context: Vec<_> = headings.last().map(|(_, id)| *id).into_iter().collect();
            if matches!(kind, "list_block" | "code_block") {
                if let Some(previous) = previous_nonblank {
                    if !context.contains(&previous) {
                        context.push(previous);
                    }
                }
            }
            let mut marks = cues(&event.text[start..offset]);
            if unclosed {
                marks.push("unclosed_fence_preserved_to_end");
            }
            spans.push(Candidate {
                id,
                event: event.id,
                start_byte: start,
                end_byte: offset,
                kind,
                context,
                cues: marks,
            });
            if let Some(level) = level {
                headings.push((level, id));
            }
            if kind != "whitespace" {
                previous_nonblank = Some(id);
            }
        }
        if event.role == "assistant" {
            prior_assistant = (first_id..spans.len()).collect();
        }
    }
    Ok(Candidates {
        status: "complete_source_partition; semantic_candidates_unverified",
        spans,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unicode_crlf_and_fenced_lists_exactly() {
        let text = "# 標題\r\n\r\n原因：\r\n```text\r\n1. not a menu\r\n```\r\n最後一句";
        let events = vec![TextEvent {
            id: 1,
            role: "assistant".into(),
            text: text.into(),
        }];
        let out = extract(&events).unwrap();
        assert_eq!(
            out.spans
                .iter()
                .map(|s| &text[s.start_byte..s.end_byte])
                .collect::<String>(),
            text
        );
        let code = out.spans.iter().find(|s| s.kind == "code_block").unwrap();
        assert!(text[code.start_byte..code.end_byte].contains("not a menu"));
        assert!(!code.context.is_empty());
    }
    #[test]
    fn numeric_reply_keeps_full_antecedent_candidate_and_quoted_cues_are_not_facts() {
        let events = vec![
            TextEvent {
                id: 1,
                role: "assistant".into(),
                text: "Choices\n\n1. a\n2. b\n".into(),
            },
            TextEvent {
                id: 2,
                role: "user".into(),
                text: "2".into(),
            },
        ];
        let out = extract(&events).unwrap();
        assert_eq!(out.spans.last().unwrap().context.len(), out.spans.len() - 1);
        assert_eq!(
            cues("The documentation says FAIL"),
            vec!["possible_negative_or_pending"]
        );
    }
    #[test]
    fn wrapped_numeric_request_retains_full_source_and_antecedent() {
        let events = vec![
            TextEvent {
                id: 1,
                role: "assistant".into(),
                text: "1. a\n2. b".into(),
            },
            TextEvent {
                id: 2,
                role: "user".into(),
                text: "<USER_REQUEST>\n2\n</USER_REQUEST>\n<META>time</META>".into(),
            },
        ];
        let result = extract(&events).unwrap();
        let user = result.spans.last().unwrap();
        assert_eq!(user.end_byte, events[1].text.len());
        assert!(!user.context.is_empty());
    }

    #[test]
    fn unclosed_fence_is_not_truncated_and_missing_antecedent_is_explicit() {
        let events = vec![
            TextEvent {
                id: 1,
                role: "user".into(),
                text: "繼續".into(),
            },
            TextEvent {
                id: 2,
                role: "assistant".into(),
                text: "```\nunfinished\n".into(),
            },
        ];
        let out = extract(&events).unwrap();
        assert!(out.spans[0].cues.contains(&"unresolved_reference"));
        assert_eq!(out.spans[1].end_byte, events[1].text.len());
        assert!(out.spans[1]
            .cues
            .contains(&"unclosed_fence_preserved_to_end"));
    }
}

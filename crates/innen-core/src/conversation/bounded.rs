use std::io::{BufRead, BufReader};

use serde_json::Value;

pub(super) struct CappedLine {
    pub(super) bytes: Vec<u8>,
    pub(super) oversized: bool,
}

pub(super) fn read_capped_line(
    reader: &mut BufReader<std::fs::File>,
    budget: usize,
) -> std::io::Result<Option<CappedLine>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            return Ok(Some(CappedLine { bytes, oversized }));
        }
        if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
            let take = pos + 1;
            if !oversized && bytes.len() + pos <= budget {
                bytes.extend_from_slice(&available[..pos]);
            } else {
                oversized = true;
            }
            reader.consume(take);
            return Ok(Some(CappedLine { bytes, oversized }));
        }
        if !oversized {
            if bytes.len() + available.len() <= budget {
                bytes.extend_from_slice(available);
            } else {
                oversized = true;
                bytes.clear();
            }
        }
        let take = available.len();
        reader.consume(take);
    }
}

pub(super) fn is_injected_envelope(text: &str) -> bool {
    let trimmed = text.trim();
    [
        ("<environment_context>", "</environment_context>"),
        ("<user_information>", "</user_information>"),
    ]
    .iter()
    .any(|(open, close)| {
        trimmed.starts_with(open)
            && trimmed.ends_with(close)
            && trimmed.len() > open.len() + close.len()
    }) || {
        let trimmed = text.trim();
        trimmed.starts_with("<INSTRUCTIONS>\n# AGENTS.md") && trimmed.ends_with("</INSTRUCTIONS>")
    }
}

pub(super) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn event_text(event: &Value) -> String {
    let content = event.pointer("/payload/content").unwrap_or(event);
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(super) fn bounded_event(mut event: Value, line: usize) -> Value {
    if event.to_string().len() <= 16 * 1024 {
        return event;
    }
    if let Some(payload) = event.get_mut("payload").and_then(Value::as_object_mut) {
        payload.insert(
            "content".into(),
            Value::String(format!(
                "[brief omitted oversized content; expand source line {line}]"
            )),
        );
        payload.insert(
            "brief_truncated_fields".into(),
            Value::Array(vec![Value::String("content".into())]),
        );
    }
    if event.to_string().len() > 16 * 1024 {
        return serde_json::json!({"brief_truncated": true, "source_line": line});
    }
    event
}

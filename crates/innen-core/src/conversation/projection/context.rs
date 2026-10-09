//! Complete human-text projection: full dialogue when it exists, an explicit
//! partial-tool fallback when it does not, and the native event otherwise.

use super::dialogue::{copy_identity, dialogue};
use super::index::index;
use crate::conversation::{sources::Source, ReadError};
use serde_json::{json, Value};

/// Complete human text and explicitly partial tool navigation in source order.
/// Other dialects retain native events until their mixed-block mapping is
/// implemented, rather than silently losing embedded tool blocks.
pub fn context(source: Source, event: Value) -> Result<Option<Value>, ReadError> {
    if source != Source::Codex {
        return Ok(Some(event));
    }
    if event["type"] == "compacted"
        || (event["type"] == "event_msg"
            && matches!(
                event["payload"]["type"].as_str(),
                Some("error" | "warning" | "turn_aborted")
            ))
    {
        return Ok(Some(event));
    }
    let mut human = dialogue(source, event.clone())?;
    if human.is_none()
        && event["type"] == "response_item"
        && event["payload"]["type"] == "message"
        && matches!(
            event["payload"]["role"].as_str(),
            Some("user" | "assistant")
        )
    {
        let mut empty = json!({"role":event["payload"]["role"],"content":""});
        copy_identity(&event, &mut empty);
        human = Some(empty);
    }
    if let Some(mut human) = human {
        let nontext: Vec<_> = event
            .pointer("/payload/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|p| {
                !matches!(
                    p["type"].as_str(),
                    Some("text" | "input_text" | "output_text")
                )
            })
            .map(|p| p.get("type").cloned().unwrap_or(json!("unknown")))
            .collect();
        if !nontext.is_empty() {
            human["nontext_parts_in_source"] = json!(nontext);
        }
        if let Some(phase) = event.pointer("/payload/phase") {
            human["phase"] = phase.clone();
        }
        Ok(Some(human))
    } else {
        let preview = index(source, event.clone())?;
        if preview.is_none()
            && event["type"] == "response_item"
            && event["payload"]["type"] != "reasoning"
            && !(event["payload"]["type"] == "message"
                && matches!(
                    event["payload"]["role"].as_str(),
                    Some("system" | "developer")
                ))
        {
            return Ok(Some(event));
        }
        Ok(preview)
    }
}

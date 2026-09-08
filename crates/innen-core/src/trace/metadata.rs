//! Explicit session descriptors are retrieval hints, not inferred provenance.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{valid_session_id, Locator};

const MAX_REFERENCES: usize = 32;
const MAX_DEPTH: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Reference {
    pub(super) locator: Locator,
    pub(super) pointer: String,
}

#[derive(Default)]
pub(super) struct Metadata {
    pub(super) references: Vec<Reference>,
    pub(super) truncated: bool,
}

impl Metadata {
    fn add(&mut self, source: &str, id: &str, pointer: String) {
        if !valid_session_id(id) {
            return;
        }
        if self.references.len() >= MAX_REFERENCES {
            self.truncated = true;
            return;
        }
        let locator = Locator::Conversation {
            source: source.into(),
            session_id: id.into(),
            source_root: None,
        };
        if !self.references.iter().any(|r| r.locator == locator) {
            self.references.push(Reference { locator, pointer });
        }
    }
}

fn walk(value: &Value, pointer: &str, depth: usize, out: &mut Metadata) {
    if depth > MAX_DEPTH {
        out.truncated = true;
        return;
    }
    if let Some(array) = value.as_array() {
        for (i, child) in array.iter().take(MAX_REFERENCES).enumerate() {
            walk(child, &format!("{pointer}/{i}"), depth + 1, out);
        }
        out.truncated |= array.len() > MAX_REFERENCES;
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    let provider = ["source", "tool", "provider"]
        .iter()
        .filter_map(|key| object.get(*key)?.as_str())
        .map(str::to_ascii_lowercase)
        .find(|s| crate::conversation::Source::parse(s).is_ok())
        .unwrap_or_else(|| "auto".into());
    for key in ["session_id", "conversation_id", "origin_session_id"] {
        if let Some(id) = object.get(key).and_then(Value::as_str) {
            out.add(&provider, id, format!("{pointer}/{key}"));
        }
    }
    for key in [
        "source",
        "sources",
        "provenance",
        "conversation",
        "conversations",
        "origin",
    ] {
        if let Some(child) = object.get(key).filter(|v| v.is_object() || v.is_array()) {
            walk(child, &format!("{pointer}/{key}"), depth + 1, out);
        }
    }
}

pub(super) fn references(text: &str) -> Metadata {
    let mut out = Metadata::default();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        walk(&value, "", 0, &mut out);
        return out;
    }
    // Only explicit descriptor lines; arbitrary UUIDs in prose are not sources.
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        for prefix in [
            "native session:",
            "native session：",
            "session_id:",
            "session_id：",
            "conversation_id:",
        ] {
            if lower.starts_with(prefix) {
                let tail = line[prefix.len()..].trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '`' | '\'' | '"')
                });
                let id = tail
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                    .next()
                    .unwrap_or("");
                out.add("auto", id, format!("/line/{}", i + 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::references;
    #[test]
    fn explicit_descriptors_only_not_arbitrary_quoted_ids() {
        let id = "00000000-0000-4000-8000-000000000001";
        assert!(references(&format!("Some unrelated example {id}"))
            .references
            .is_empty());
        let result = references(&format!(r#"{{"source":"codex","session_id":"{id}"}}"#));
        assert_eq!(result.references.len(), 1);
        assert_eq!(result.references[0].pointer, "/session_id");
        assert_eq!(
            references(&format!("native session：`{id}`。startup evidence"))
                .references
                .len(),
            1
        );
    }
}

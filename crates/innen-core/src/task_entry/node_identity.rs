//! Node text extraction: searchable text for indexing, and the identity,
//! anchor, and display-kind views every admission gate reads.

use serde_json::Value;

use crate::graph::NodeType;

/// Helper to extract text from a node.
pub(super) fn node_text<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Extract display kind from node.
pub(super) fn node_kind(node: &Value) -> String {
    match node.get("type").and_then(|v| v.as_str()) {
        Some(s) => s.parse::<NodeType>().unwrap().to_string(),
        None => "Custom".to_string(),
    }
}

/// Helper to concatenate all relevant metadata (label, aliases, summary, body, path)
/// for indexing into Tantivy.
pub fn node_searchable_text(node: &Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let label = node_text(node, "label");
    if !label.is_empty() {
        parts.push(label);
    }
    let name = node_text(node, "name");
    if !name.is_empty() {
        parts.push(name);
    }
    let body = node_text(node, "body");
    if !body.is_empty() {
        parts.push(body);
    }
    if let Some(s) = node.get("summary").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    if let Some(arr) = node.get("summary").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }
    }
    if let Some(s) = node.get("aliases").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    if let Some(arr) = node.get("aliases").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }
    }
    if let Some(s) = node.get("alias").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    let path = node_text(node, "path");
    if !path.is_empty() {
        parts.push(path);
    }
    for key in ["document_id", "revision_id"] {
        let value = node_text(node, key);
        if !value.is_empty() {
            parts.push(value);
        }
    }
    parts.join(" ")
}

/// Identity-bearing fields used to admit a task candidate. Body text is kept
/// for evidence search, but a generic body term must not make an unrelated
/// node a candidate for a named task.
pub(super) fn node_identity_text(node: &Value) -> String {
    let mut parts = Vec::new();
    for key in [
        "label",
        "name",
        "aliases",
        "alias",
        "path",
        "document_id",
        "revision_id",
    ] {
        match node.get(key) {
            Some(Value::String(s)) if !s.is_empty() => parts.push(s.clone()),
            Some(Value::Array(values)) => parts.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    parts.join(" ")
}

pub(super) fn node_anchor_text(node: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["label", "name", "aliases", "alias"] {
        match node.get(key) {
            Some(Value::String(s)) if !s.is_empty() => parts.push(s.clone()),
            Some(Value::Array(values)) => parts.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    parts.join(" ")
}

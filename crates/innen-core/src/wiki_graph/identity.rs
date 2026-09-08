use super::types::{WikiGraphError, WikiPage, MANAGED_BY};
use crate::graph::Materialized;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub(super) fn assign_ids(
    pages: &mut [WikiPage],
    graph: &Materialized,
) -> Result<(), WikiGraphError> {
    let mut registered: HashMap<String, String> = HashMap::new();
    for (id, node) in &graph.nodes {
        if node.get("managed_by").and_then(Value::as_str) != Some(MANAGED_BY)
            || node.get("type").and_then(Value::as_str) != Some("Wiki")
        {
            continue;
        }
        let Some(path) = node.get("wiki_path").and_then(Value::as_str) else {
            continue;
        };
        if let Some(previous) = registered.insert(path.to_string(), id.clone()) {
            if previous != *id {
                return Err(WikiGraphError::Identity(format!(
                    "registered wiki path {path:?} has multiple ids: {previous}, {id}"
                )));
            }
        }
    }
    let mut claimed = HashMap::new();
    for page in pages {
        page.id = registered.get(&page.wiki_path).cloned().unwrap_or_else(|| {
            page.explicit_id.clone().unwrap_or_else(|| {
                format!("wiki:{}", crate::ids::sha256_hex(page.wiki_path.as_bytes()))
            })
        });
        if let Some(previous_path) = claimed.insert(page.id.clone(), page.wiki_path.clone()) {
            return Err(WikiGraphError::Identity(format!(
                "wiki id {:?} is claimed by both {:?} and {:?}",
                page.id, previous_path, page.wiki_path
            )));
        }
    }
    Ok(())
}

pub(super) fn wiki_node(page: &WikiPage) -> Value {
    json!({
        "id": page.id,
        "type": "Wiki",
        "title": page.title,
        "label": page.title,
        "tags": page.tags,
        "body": page.body,
        "path": page.path.to_string_lossy(),
        "wiki_path": page.wiki_path,
        "sha256": page.sha256,
        "missing": false,
        "managed_by": MANAGED_BY,
        "provenance": format!("wiki:{}#sha256={}", page.path.display(), page.sha256),
    })
}

pub(super) fn source_node(source_ref: &str, locator: Value) -> (String, Value) {
    let hash = crate::ids::sha256_hex(source_ref.as_bytes());
    let id = format!("source:{hash}");
    let value = json!({
        "id": id,
        "type": "Source",
        "label": source_ref,
        "source_ref": source_ref,
        "locator": locator,
        "managed_by": MANAGED_BY,
        "provenance": format!("source-ref:sha256={hash}"),
    });
    (id, value)
}

pub(super) fn changed_fields(current: Option<&Value>, desired: &Value) -> Option<Value> {
    let desired = desired.as_object()?;
    let mut changed = Map::new();
    changed.insert("id".to_string(), desired.get("id")?.clone());
    let current = current.and_then(Value::as_object);
    for (key, value) in desired {
        if key == "id" {
            continue;
        }
        if current.and_then(|node| node.get(key)) != Some(value) {
            changed.insert(key.clone(), value.clone());
        }
    }
    (changed.len() > 1 || current.is_none()).then_some(Value::Object(changed))
}

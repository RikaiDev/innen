//! Bounded, typed relationship traversal from admitted candidates: status,
//! baseline approval, owning projects, and informing decisions.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Value};

use crate::graph::{EdgeType, StoredEdge};

use super::node_identity::node_text;

/// Check if a filename is an editable vector/code baseline.
fn is_editable_format(label: &str, path: &str) -> bool {
    let target = if !label.is_empty() { label } else { path };
    let lower = target.to_lowercase();
    lower.ends_with(".svg")
        || lower.ends_with(".ai")
        || lower.ends_with(".eps")
        || lower.ends_with(".typ")
        || lower.ends_with(".fig")
        || lower.ends_with(".sketch")
}

/// What one traversal yields: project links, connected decisions, per-candidate
/// status, and the approved subset with its (id, label, status) triple.
pub(super) type TraversalOutput = (
    BTreeSet<String>,
    Vec<Value>,
    BTreeMap<String, String>,
    Vec<(String, String, String)>,
);

/// Walk the ranked candidates once, recording display status, the approved
/// editable baseline, explicit project ownership, and informing decisions.
pub(super) fn traverse_candidates(
    ranked_candidates: &[(String, f64, String)],
    nodes: &HashMap<String, Value>,
    edges: &[StoredEdge],
) -> TraversalOutput {
    let mut linked_projects: BTreeSet<String> = BTreeSet::new();
    let mut connected_decisions: Vec<Value> = Vec::new();
    let mut candidate_status_map: BTreeMap<String, String> = BTreeMap::new();
    let mut approved_candidates: Vec<(String, String, String)> = Vec::new(); // (id, label, status)

    for (id, _, _) in ranked_candidates {
        let node = &nodes[id];
        let raw_status = node_text(node, "status");
        let label = node_text(node, "label");
        let path = node_text(node, "path");

        // Missing status must be unknown, never fabricated draft
        let display_status = if raw_status.trim().is_empty() {
            "unknown".to_string()
        } else {
            raw_status.to_string()
        };
        candidate_status_map.insert(id.clone(), display_status.clone());

        // Check explicit approval (status=approved; not just final, and has verified location)
        let has_explicit_approval = display_status.eq_ignore_ascii_case("approved");
        let has_source_location = node
            .get("location_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || node
                .get("location_verified")
                .and_then(Value::as_str)
                .is_some_and(|v| v.eq_ignore_ascii_case("verified"));

        if has_explicit_approval && is_editable_format(label, path) && has_source_location {
            approved_candidates.push((id.clone(), label.to_string(), display_status));
        }

        // Inspect edges from candidate: ONLY explicit BELONGS_TO edges to a Project node
        for edge in edges {
            if edge.retracted {
                continue;
            }
            if edge.from == *id && edge.edge == EdgeType::BelongsTo {
                if let Some(to_node) = nodes.get(&edge.to) {
                    if node_text(to_node, "type").eq_ignore_ascii_case("project") {
                        linked_projects.insert(edge.to.clone());
                    }
                }
            }
            if (edge.from == *id || edge.to == *id)
                && matches!(edge.edge, EdgeType::Supports | EdgeType::Informs)
            {
                let other_id = if edge.from == *id {
                    &edge.to
                } else {
                    &edge.from
                };
                if let Some(dec_node) = nodes.get(other_id) {
                    if node_text(dec_node, "type").eq_ignore_ascii_case("decision") {
                        connected_decisions.push(json!({
                            "id": other_id,
                            "label": node_text(dec_node, "label"),
                            "status": node_text(dec_node, "status"),
                        }));
                    }
                }
            }
        }
    }

    (
        linked_projects,
        connected_decisions,
        candidate_status_map,
        approved_candidates,
    )
}

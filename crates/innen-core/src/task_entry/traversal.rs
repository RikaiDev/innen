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

        // A candidate that is itself a project is its own project link; the
        // BELONGS_TO walk below only finds the owner of a non-project node.
        if node_text(node, "type").eq_ignore_ascii_case("project") {
            linked_projects.insert(id.clone());
        }

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

/// Most siblings listed for the linked projects; the total is reported so a
/// truncated list is never mistaken for the whole project.
pub(super) const NEIGHBORHOOD_LIMIT: usize = 12;

/// Other nodes owned by the linked projects (decks, proposals, pages), so one
/// query shows what already exists for the task. Returns the bounded rows and
/// the full count.
pub(super) fn project_neighbors(
    linked_projects: &BTreeSet<String>,
    nodes: &HashMap<String, Value>,
    edges: &[StoredEdge],
    seeds: &BTreeSet<&str>,
) -> (Vec<Value>, usize) {
    let mut rows: Vec<(String, Value)> = Vec::new();
    for edge in edges {
        if edge.retracted
            || edge.edge != EdgeType::BelongsTo
            || !linked_projects.contains(&edge.to)
            || seeds.contains(edge.from.as_str())
        {
            continue;
        }
        let Some(node) = nodes.get(&edge.from) else {
            continue;
        };
        if is_constraint(node) {
            continue;
        }
        let label = node_text(node, "label").to_string();
        rows.push((
            label.clone(),
            json!({"id": edge.from, "kind": super::node_identity::node_kind(node), "label": label, "project": edge.to}),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.dedup_by(|a, b| a.1["id"] == b.1["id"]);
    let total = rows.len();
    let shown = rows
        .into_iter()
        .take(NEIGHBORHOOD_LIMIT)
        .map(|(_, v)| v)
        .collect();
    (shown, total)
}

/// What the best-scored candidates own: their projects, and the project's
/// other files when the request named a project itself.
pub(super) struct Focus {
    pub projects: BTreeSet<String>,
    pub neighborhood: Vec<Value>,
    pub neighborhood_total: usize,
    pub constraints: Vec<Value>,
    pub constraints_total: usize,
}

/// The neighborhood follows only the best-scored candidates: a project reached
/// through one weak generic term must not list its files as if the request
/// were about it. A request that already hit concrete files does not need the
/// project's other files, so siblings are listed only for a project candidate.
pub(super) fn best_scored_focus(
    ranked: &[(String, f64, String)],
    nodes: &HashMap<String, Value>,
    edges: &[StoredEdge],
    terms: &[String],
) -> Focus {
    let best = ranked.first().map_or(0.0, |c| c.1);
    let top: Vec<(String, f64, String)> = ranked
        .iter()
        .filter(|c| c.1 >= best - 1e-9)
        .cloned()
        .collect();
    let (projects, ..) = traverse_candidates(&top, nodes, edges);
    let names_a_project = top.iter().any(|c| {
        nodes
            .get(&c.0)
            .is_some_and(|n| node_text(n, "type").eq_ignore_ascii_case("project"))
    });
    let scope = if names_a_project {
        projects.clone()
    } else {
        BTreeSet::new()
    };
    let seeds: BTreeSet<&str> = ranked.iter().map(|c| c.0.as_str()).collect();
    let (neighborhood, neighborhood_total) = project_neighbors(&scope, nodes, edges, &seeds);
    let (constraints, constraints_total) = project_constraints(&projects, nodes, edges, terms);
    Focus {
        projects,
        neighborhood,
        neighborhood_total,
        constraints,
        constraints_total,
    }
}

/// Most constraints listed per request; the total is reported alongside.
pub(super) const CONSTRAINT_LIMIT: usize = 12;

fn is_constraint(node: &Value) -> bool {
    node_text(node, "type").eq_ignore_ascii_case("constraint")
}

/// The user's recorded binding requirements for the given projects: the
/// verbatim `body` and the `provenance` that locates it in its source. The
/// text is what an agent recorded, so it is reported with its source and never
/// as verified.
pub(super) fn project_constraints(
    projects: &BTreeSet<String>,
    nodes: &HashMap<String, Value>,
    edges: &[StoredEdge],
    terms: &[String],
) -> (Vec<Value>, usize) {
    let lowered: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut rows: Vec<(usize, String, Value)> = Vec::new();
    for edge in edges {
        if edge.retracted || edge.edge != EdgeType::BelongsTo || !projects.contains(&edge.to) {
            continue;
        }
        let Some(node) = nodes.get(&edge.from).filter(|n| is_constraint(n)) else {
            continue;
        };
        // Under a limit, relevance decides what survives: rank by how many
        // query terms the words contain, then by id for a stable order.
        let hay =
            format!("{} {}", node_text(node, "body"), node_text(node, "label")).to_lowercase();
        let hits = lowered.iter().filter(|t| hay.contains(t.as_str())).count();
        rows.push((
            hits,
            edge.from.clone(),
            json!({
                "id": edge.from,
                "text": node_text(node, "body"),
                "source": node_text(node, "provenance"),
            }),
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    rows.dedup_by(|a, b| a.1 == b.1);
    let total = rows.len();
    let shown = rows
        .into_iter()
        .take(CONSTRAINT_LIMIT)
        .map(|(_, _, v)| v)
        .collect();
    (shown, total)
}

//! Admission and scoring gates: which nodes a request may select, and how a
//! literal identity match is scored against the whole request.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use super::node_identity::{node_anchor_text, node_identity_text, node_searchable_text, node_text};

pub(super) fn has_entity_anchor(node: &Value, entity_terms: &[&str]) -> bool {
    if entity_terms.is_empty() {
        return false;
    }
    let anchor = node_anchor_text(node).to_lowercase();
    // A shared workspace/path label is location evidence, not brand/project
    // identity. It must not select the named entity by itself.
    if anchor.contains("workspace:") || anchor.contains('/') {
        return false;
    }
    let matched = entity_terms
        .iter()
        .filter(|term| anchor.contains(**term))
        .count();
    // A single identity term is enough for a named item such as `weemed`.
    // For a multi-term query, one generic word such as `agent` is not an
    // identity anchor; require the complete anchor instead.
    matched > 0 && (entity_terms.len() == 1 || matched == entity_terms.len())
}

pub(super) fn has_all_search_terms(node: &Value, terms: &[&str]) -> bool {
    if terms.is_empty() {
        return false;
    }
    let searchable = node_searchable_text(node).to_lowercase();
    terms.iter().all(|term| searchable.contains(term))
}

pub(super) fn admit_literal_candidate(node: &Value, terms: &[&str], allow_partial: bool) -> bool {
    allow_partial || terms.len() <= 1 || has_all_search_terms(node, terms)
}

/// Score literal identity matches by how much of the meaningful request they
/// cover.  The old fixed 0.85 score made every node containing one generic
/// phrase indistinguishable across projects.
fn identity_coverage_score(node: &Value, terms: &[&str], entity_terms: &[&str]) -> f64 {
    let fields = [
        "label",
        "name",
        "aliases",
        "alias",
        "path",
        "document_id",
        "revision_id",
    ];
    let mut matched = 0usize;
    let mut weighted = 0.0;
    for term in terms {
        let t = term.to_lowercase();
        let mut best: f64 = 0.0;
        for key in fields {
            let hit = match node.get(key) {
                Some(Value::String(s)) => s.to_lowercase().contains(&t),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|s| s.to_lowercase().contains(&t)),
                _ => false,
            };
            if hit {
                best = best.max(if key == "label" || key == "name" {
                    1.0
                } else {
                    0.8
                });
            }
        }
        if best > 0.0 {
            matched += 1;
            weighted += best;
        }
    }
    if terms.is_empty() {
        return 0.0;
    }
    let coverage = weighted / terms.len() as f64;
    let anchor = if entity_terms
        .iter()
        .any(|t| node_anchor_text(node).to_lowercase().contains(t))
    {
        0.2
    } else {
        0.0
    };
    (0.5 + 0.5 * coverage + anchor).min(1.0) * (matched as f64 / terms.len() as f64).sqrt()
}

/// Literal substring matching on identity fields. FTS is broad by design: a
/// single generic identity term is noise, while a conjunctive match for the
/// complete request is a useful topic candidate. Scores are merged into the
/// seeds already admitted from FTS.
pub(super) fn literal_candidate_scores(
    nodes: &HashMap<String, Value>,
    terms: &[String],
    identity_terms: &[&str],
    entity_terms: &[&str],
    q_lower: &str,
    allow_partial_identity: bool,
    candidate_scores: &mut BTreeMap<String, (f64, String)>,
) {
    if !q_lower.is_empty() {
        for (id, node) in nodes {
            let label = node_text(node, "label").to_lowercase();
            let id_lower = id.to_lowercase();
            let identity = node_identity_text(node).to_lowercase();
            let path_like_label = label.contains("workspace:") || label.contains('/');

            let mut literal_matched = false;
            let mut exact_match = false;
            // An id is the one string guaranteed to identify a node. When the
            // query *is* that id, the all-terms gate below must not apply: it
            // splits the query into parts and then looks for those parts in
            // label/body, which an id never appears in.
            let exact_id_match = !q_lower.is_empty() && id_lower == *q_lower;

            // An exact id match is a seed in its own right. It must not depend
            // on the literal substring test below, which skips the id branch
            // whenever the label looks path-like (`workspace:`, or any `/`):
            // a node labelled "session/repo" would otherwise be unfindable by
            // its own id, and the all-terms gate would never even be consulted.
            if exact_id_match {
                literal_matched = true;
                exact_match = true;
            }

            // Check exact query match (min 2 chars)
            if q_lower.chars().count() >= 2
                && (label.contains(q_lower) || (!path_like_label && id_lower.contains(q_lower)))
            {
                literal_matched = true;
                exact_match = true;
            }

            // Check hyphenated tokens or extracted significant terms
            for term in terms {
                if !entity_terms.is_empty() && !entity_terms.contains(&term.as_str()) {
                    continue;
                }
                if term.len() >= 2
                    && !path_like_label
                    && (entity_terms.len() <= 1
                        || allow_partial_identity
                        || entity_terms
                            .iter()
                            .all(|entity| label.contains(entity) || id_lower.contains(entity)))
                    && (label.contains(term) || id_lower.contains(term))
                {
                    literal_matched = true;
                    if label == *term || id_lower == *term {
                        exact_match = true;
                    }
                }
            }

            let identity_match = if entity_terms.is_empty() {
                identity_terms.iter().any(|term| identity.contains(term))
            } else {
                has_entity_anchor(node, entity_terms)
            };
            if identity_match {
                literal_matched = true;
            }

            // A single common word (for example 報價) must not admit an
            // unrelated asset for a multi-part request. Keep partial matches
            // only for explicit asset-edit queries, where browsing variants
            // is intentional.
            if literal_matched
                && !exact_id_match
                && !admit_literal_candidate(node, identity_terms, allow_partial_identity)
            {
                literal_matched = false;
            }

            if literal_matched {
                let score = if exact_match || exact_id_match {
                    1.0
                } else {
                    identity_coverage_score(node, identity_terms, entity_terms)
                };
                let current = candidate_scores
                    .entry(id.clone())
                    .or_insert((0.0, String::new()));
                if score > current.0 {
                    current.0 = score;
                    current.1 = "literal_match".to_string();
                }
            }
        }
    }
}

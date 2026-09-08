use super::cache::{locator, GraphCache, Link};
use super::lexical::bm25;
use super::types::{Error, Locator, Options};
use crate::graph::{validate, EdgeType, Endpoint, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_FRONTIER: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Hop {
    pub(super) from: String,
    pub(super) edge: String,
    pub(super) to: String,
    pub(super) direction: String,
    pub(super) provenance: Option<String>,
}
#[derive(Clone)]
struct Frontier {
    node: String,
    seed: String,
    score: f64,
    path: Vec<Hop>,
}
#[derive(Clone)]
pub(super) struct Candidate {
    pub(super) node: String,
    pub(super) seed: String,
    pub(super) score: f64,
    pub(super) path: Vec<Hop>,
    pub(super) locator: Locator,
}
struct Step {
    target: String,
    weight: f64,
    hop: Hop,
}

fn weight(kind: &str, reverse: bool) -> Option<f64> {
    match (kind, reverse) {
        (
            "DERIVED_FROM" | "DISCUSSED_IN" | "DECIDED_IN" | "LOCATED_AT" | "ORIGINATED_AT",
            false,
        ) => Some(0.9),
        ("DELIVERED", true) => Some(0.9),
        ("SUPERSEDES" | "FOLLOWS_UP", _) => Some(0.65),
        ("RELATED_TO", _) => Some(0.35),
        ("BELONGS_TO", _) => Some(0.12),
        _ => None,
    }
}

fn valid_link_at(graph: &GraphCache, link: &Link, now: &str) -> bool {
    if link.valid_from.as_str() > now || link.valid_until.as_deref().is_some_and(|end| now > end) {
        return false;
    }
    let Some(from) = graph.nodes.get(&link.from) else {
        return false;
    };
    let from_type: NodeType = from["type"].as_str().unwrap_or("Custom").parse().unwrap();
    let endpoint = if let Some(to) = graph.nodes.get(&link.to) {
        Endpoint::Node(to["type"].as_str().unwrap_or("Custom").parse().unwrap())
    } else if crate::graph::is_uri_shape(&link.to) {
        Endpoint::Uri(link.to.clone())
    } else {
        return false;
    };
    let edge: EdgeType = link.kind.parse().unwrap();
    let provenance = link
        .provenance
        .as_deref()
        .or_else(|| from["provenance"].as_str());
    validate(&from_type, &edge, &endpoint, provenance).is_ok()
}

fn adjacency(graph: &GraphCache) -> BTreeMap<String, Vec<Step>> {
    let mut out = BTreeMap::<String, Vec<Step>>::new();
    let now = crate::journal::observed_utc_now();
    for link in &graph.links {
        if !valid_link_at(graph, link, &now) {
            continue;
        }
        for reverse in [false, true] {
            let Some(weight) = weight(&link.kind, reverse) else {
                continue;
            };
            let (origin, target) = if reverse {
                (&link.to, &link.from)
            } else {
                (&link.from, &link.to)
            };
            out.entry(origin.clone()).or_default().push(Step {
                target: target.clone(),
                weight,
                hop: Hop {
                    from: link.from.clone(),
                    edge: link.kind.clone(),
                    to: link.to.clone(),
                    direction: if reverse { "reverse" } else { "forward" }.into(),
                    provenance: link.provenance.clone(),
                },
            });
        }
    }
    for steps in out.values_mut() {
        steps.sort_by(|a, b| {
            b.weight
                .total_cmp(&a.weight)
                .then_with(|| a.target.cmp(&b.target))
        });
    }
    out
}

fn seeds(graph: &GraphCache, opts: &Options, query: &[String]) -> Result<Vec<Frontier>, Error> {
    if let Some(seed) = &opts.seed {
        if !graph.nodes.contains_key(seed) {
            return Err(Error(format!("unknown explicit seed {seed}")));
        }
        return Ok(vec![Frontier {
            node: seed.clone(),
            seed: seed.clone(),
            score: 1.0,
            path: Vec::new(),
        }]);
    }
    let ranked = bm25(&graph.docs, query);
    let now = crate::journal::observed_utc_now();
    let maximum = ranked.first().map(|r| r.1).unwrap_or(1.0);
    Ok(ranked
        .into_iter()
        .filter(|(i, _)| {
            let id = &graph.docs[*i].id;
            let node = &graph.nodes[id];
            node["managed_by"] != crate::wiki_graph::MANAGED_BY
                || node["type"] != "Source"
                || graph.links.iter().any(|link| {
                    link.to == *id
                        && link.kind == "DERIVED_FROM"
                        && valid_link_at(graph, link, &now)
                })
        })
        .take(12)
        .map(|(i, score)| {
            let id = graph.docs[i].id.clone();
            Frontier {
                node: id.clone(),
                seed: id,
                score: score / maximum,
                path: Vec::new(),
            }
        })
        .collect())
}

fn candidate(graph: &GraphCache, root: &Path, current: &Frontier) -> Option<Candidate> {
    let loc = if let Some(node) = graph.nodes.get(&current.node) {
        locator(node, root)
    } else if current.node.starts_with('/') {
        Some(Locator::File {
            path: current.node.clone(),
        })
    } else if current.node.starts_with("https://") || current.node.starts_with("http://") {
        Some(Locator::Url {
            url: current.node.clone(),
        })
    } else {
        None
    }?;
    Some(Candidate {
        node: current.node.clone(),
        seed: current.seed.clone(),
        score: current.score,
        path: current.path.clone(),
        locator: loc,
    })
}

fn advance(
    current: &Frontier,
    steps: &[Step],
    visited: &BTreeSet<String>,
    pending: &mut Vec<Frontier>,
) -> bool {
    let mut limited = false;
    for step in steps {
        if visited.contains(&step.target) {
            continue;
        }
        if pending.len() >= MAX_FRONTIER {
            limited = true;
            break;
        }
        let hub = if step.hop.edge == "BELONGS_TO" {
            (steps.len() as f64).sqrt().max(1.0)
        } else {
            1.0
        };
        let mut path = current.path.clone();
        path.push(step.hop.clone());
        pending.push(Frontier {
            node: step.target.clone(),
            seed: current.seed.clone(),
            score: current.score * step.weight / hub,
            path,
        });
    }
    limited
}

/// Best-first search preserves seed rank and produces an explicit path witness.
pub(super) fn candidates(
    graph: &GraphCache,
    root: &Path,
    opts: &Options,
    query: &[String],
) -> Result<(Vec<Candidate>, usize, bool), Error> {
    let mut pending = seeds(graph, opts, query)?;
    let edges = adjacency(graph);
    let mut visited = BTreeSet::new();
    let mut found = Vec::new();
    let mut limited = false;
    while !pending.is_empty() && visited.len() < opts.max_nodes {
        pending.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node.cmp(&b.node))
        });
        let current = pending.remove(0);
        if !visited.insert(current.node.clone()) {
            continue;
        }
        if let Some(hit) = candidate(graph, root, &current) {
            found.push(hit);
        }
        let Some(steps) = edges.get(&current.node) else {
            continue;
        };
        if current.path.len() >= opts.max_depth {
            limited |= steps.iter().any(|s| !visited.contains(&s.target));
        } else {
            limited |= advance(&current, steps, &visited, &mut pending);
        }
    }
    found.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.node.cmp(&b.node))
    });
    let mut seen = BTreeSet::new();
    found.retain(|c| seen.insert(serde_json::to_string(&c.locator).expect("locator serializes")));
    Ok((found, visited.len(), limited || !pending.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::{valid_link_at, GraphCache, Link};
    use serde_json::json;

    #[test]
    fn cached_edges_expire_without_a_journal_change() {
        let graph = GraphCache {
            schema: super::super::types::CACHE_SCHEMA,
            journal_sha256: "fixture".into(),
            nodes: [
                ("task:one".into(), json!({"type":"Task"})),
                ("project:one".into(), json!({"type":"Project"})),
            ]
            .into_iter()
            .collect(),
            links: Vec::new(),
            docs: Vec::new(),
        };
        let link = Link {
            from: "task:one".into(),
            to: "project:one".into(),
            kind: "BELONGS_TO".into(),
            provenance: None,
            valid_from: "2020-01-01T00:00:00Z".into(),
            valid_until: Some("2030-01-01T00:00:00Z".into()),
        };
        assert!(valid_link_at(&graph, &link, "2026-01-01T00:00:00Z"));
        assert!(!valid_link_at(&graph, &link, "2040-01-01T00:00:00Z"));
    }
}

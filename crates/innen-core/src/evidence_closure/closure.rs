use super::contract::{timestamp, Contract};
use super::snapshot::{edge_receipt, Snapshot};
use crate::ids::sha256_hex;
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone)]
pub(super) struct Alternative {
    pub(super) roots: BTreeSet<String>,
    pub(super) nodes: BTreeSet<String>,
    pub(super) paths: Vec<Value>,
}

pub(super) fn eligible(id: &str, c: &Contract, s: &Snapshot) -> Result<(), &'static str> {
    if !c.scope_nodes.contains(id) {
        return Err("scope_or_source_unavailable");
    }
    let grant = c.sources.get(id).ok_or("source_pin_missing")?;
    let node = s.graph.nodes.get(id).ok_or("scope_or_source_unavailable")?;
    if sha256_hex(node.to_string().as_bytes()) != grant.node_sha256.to_lowercase() {
        return Err("source_changed");
    }
    let at = node["observed_utc"]
        .as_str()
        .filter(|s| timestamp(s))
        .ok_or("observation_time_unknown")?;
    if at < c.min_observed_utc.as_str() || at > c.as_of.as_str() {
        return Err("observation_outside_window");
    }
    if node["provenance"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("source_provenance_missing");
    }
    Ok(())
}

pub(super) fn closure(
    roots: &BTreeSet<String>,
    c: &Contract,
    s: &Snapshot,
) -> Result<Alternative, (&'static str, Option<String>)> {
    let problem = |reason, id: &str| (reason, c.scope_nodes.contains(id).then(|| id.to_owned()));
    let mut nodes = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut paths = Vec::new();
    let mut seen_edges = BTreeSet::new();
    for id in roots {
        queue.push_back((id.clone(), 0usize));
    }
    while let Some((id, depth)) = queue.pop_front() {
        if nodes.contains(&id) {
            continue;
        }
        eligible(&id, c, s).map_err(|r| problem(r, &id))?;
        nodes.insert(id.clone());
        for &index in s.adjacency.get(&id).into_iter().flatten() {
            let e = &s.graph.edges[index];
            let kind = e.edge.to_string();
            if kind == "CONTRADICTS" && (e.from == id || e.to == id) {
                let other = if e.from == id { &e.to } else { &e.from };
                if !c.scope_nodes.contains(other) {
                    return Err(("scope_or_source_unavailable", None));
                }
                return Err(problem("unresolved_conflict", other));
            }
            if e.from != id || kind != "DEPENDS_ON" {
                continue;
            }
            // Permission/source/provenance checks apply even to already-visited cycle edges.
            eligible(&e.to, c, s).map_err(|r| problem(r, &e.to))?;
            if e.provenance.as_ref().is_none_or(|s| s.trim().is_empty()) {
                return Err(problem("dependency_provenance_missing", &id));
            }
            if depth >= c.max_hops && !nodes.contains(&e.to) {
                return Err(problem("hop_limit", &id));
            }
            let mut receipt = edge_receipt(e);
            let citations = s
                .edge_citations
                .get(&receipt.to_string())
                .ok_or_else(|| problem("dependency_trace_missing", &id))?;
            receipt["journal_citations"] = json!(citations);
            if seen_edges.insert(receipt.to_string()) {
                paths.push(receipt);
            }
            if !nodes.contains(&e.to) {
                queue.push_back((e.to.clone(), depth + 1));
            }
        }
    }
    Ok(Alternative {
        roots: roots.clone(),
        nodes,
        paths,
    })
}

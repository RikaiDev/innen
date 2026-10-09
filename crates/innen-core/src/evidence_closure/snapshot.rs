use super::contract::timestamp;
use crate::graph::{materialize, Materialized, StoredEdge};
use crate::ids::sha256_hex;
use crate::journal::JournalEntry;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) struct Snapshot {
    pub(super) graph: Materialized,
    pub(super) hash: String,
    pub(super) citations: BTreeMap<String, Vec<Value>>,
    pub(super) edge_citations: BTreeMap<String, Vec<Value>>,
    pub(super) adjacency: BTreeMap<String, Vec<usize>>,
}

pub(super) fn snapshot(raw: &[u8], cutoff: &str) -> Result<Snapshot, String> {
    let text = std::str::from_utf8(raw).map_err(|_| "journal is not UTF-8")?;
    let mut values = Vec::new();
    let mut citations = BTreeMap::<String, Vec<Value>>::new();
    let mut edge_citations = BTreeMap::<String, Vec<Value>>::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut e: JournalEntry = serde_json::from_str(line)
            .map_err(|_| format!("invalid journal record at line {}", i + 1))?;
        if !timestamp(&e.observed_utc) {
            return Err(format!("noncanonical journal time at line {}", i + 1));
        }
        if e.observed_utc.as_str() > cutoff {
            continue;
        }
        if matches!(e.op.as_str(), "edge.assert" | "edge.retract") {
            if ["from", "to", "type"]
                .iter()
                .any(|k| e.payload[k].as_str().is_none())
            {
                return Err("malformed graph edge".into());
            }
            let kind = e.payload["type"].as_str().unwrap().to_ascii_uppercase();
            if matches!(kind.as_str(), "DEPENDS_ON" | "CONTRADICTS") {
                e.payload["type"] = json!(kind);
            }
            for k in ["valid_from", "valid_until", "observed_utc"] {
                if let Some(v) = e.payload.get(k).filter(|v| !v.is_null()) {
                    if !v.as_str().is_some_and(timestamp) {
                        return Err("invalid edge validity timestamp".into());
                    }
                }
            }
        }
        if !matches!(
            e.op.as_str(),
            "node.upsert" | "edge.assert" | "edge.retract" | "config.set"
        ) {
            return Err(format!("unsupported journal operation at line {}", i + 1));
        }
        if e.op == "node.upsert" {
            let id = e.payload["id"].as_str().ok_or("node identity missing")?;
            citations
                .entry(id.into())
                .or_default()
                .push(json!({"line":i+1,"event":e.id,"at":e.observed_utc}));
        }
        let event = json!({"op":e.op,"payload":e.payload,"observed_utc":e.observed_utc});
        if e.op == "edge.assert" {
            let single = materialize(std::slice::from_ref(&event), Some(cutoff), true);
            if let Some(edge) = single.edges.first() {
                edge_citations
                    .entry(edge_receipt(edge).to_string())
                    .or_default()
                    .push(json!({"line":i+1,"event":e.id,"at":e.observed_utc}));
            }
        }
        values.push(event);
    }
    let graph = materialize(&values, Some(cutoff), false);
    let mut adjacency = BTreeMap::<String, Vec<usize>>::new();
    for (i, e) in graph.edges.iter().enumerate().filter(|(_, e)| !e.retracted) {
        match e.edge.to_string().as_str() {
            "DEPENDS_ON" => adjacency.entry(e.from.clone()).or_default().push(i),
            "CONTRADICTS" => {
                adjacency.entry(e.from.clone()).or_default().push(i);
                if e.from != e.to {
                    adjacency.entry(e.to.clone()).or_default().push(i);
                }
            }
            _ => {}
        }
    }
    Ok(Snapshot {
        graph,
        hash: sha256_hex(raw),
        citations,
        edge_citations,
        adjacency,
    })
}

pub(super) fn edge_receipt(e: &StoredEdge) -> Value {
    json!({"from":e.from,"to":e.to,"type":e.edge.to_string(),"observed_utc":e.observed_utc,
           "valid_from":e.valid_from,"valid_until":e.valid_until,"provenance":e.provenance})
}

//! Journal replay into a materialized graph snapshot, plus the shared URI shape rule.

use super::adjacency::EdgeType;

/// One asserted edge row. `retracted` rows are retained (never deleted).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEdge {
    pub from: String,
    pub edge: EdgeType,
    pub to: String,
    pub weight: Option<f32>,
    pub valid_from: String,
    pub valid_until: Option<String>,
    pub retracted: bool,
    pub observed_utc: String,
    /// Provenance attached to this edge assertion, when present.
    pub provenance: Option<String>,
}

/// Replay result: merged nodes by id plus edge rows in assertion order.
#[derive(Debug, Clone, Default)]
pub struct Materialized {
    pub nodes: std::collections::HashMap<String, serde_json::Value>,
    pub edges: Vec<StoredEdge>,
}

fn project_key(value: &str) -> String {
    let normalized = value.trim().trim_end_matches('/').replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    lower
        .split_once("/workspace/")
        .map_or(lower.as_str(), |(_, relative)| relative)
        .trim_start_matches("./")
        .to_string()
}

/// Resolve human project references without guessing across ambiguous matches.
/// Exact IDs win, followed by namespace-free slugs/paths, labels, then basename.
pub fn resolve_project_id(graph: &Materialized, input: &str) -> Result<String, String> {
    let query = project_key(input);
    let bare_query = ["project:", "workspace:", "p:"]
        .into_iter()
        .find_map(|prefix| query.strip_prefix(prefix))
        .unwrap_or(&query);
    let mut ranked: std::collections::BTreeMap<u8, Vec<String>> = std::collections::BTreeMap::new();
    for (id, node) in &graph.nodes {
        if !node["type"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("project"))
        {
            continue;
        }
        let id_key = project_key(id);
        let bare_id = ["project:", "workspace:", "p:"]
            .into_iter()
            .find_map(|prefix| id_key.strip_prefix(prefix))
            .unwrap_or(&id_key);
        let label = project_key(node["label"].as_str().unwrap_or(""));
        let path = project_key(node["path"].as_str().unwrap_or(""));
        let rank = if query == id_key {
            Some(0)
        } else if bare_query == bare_id || (!path.is_empty() && bare_query == path) {
            Some(1)
        } else if !label.is_empty() && query == label {
            Some(2)
        } else if bare_id.rsplit('/').next() == Some(bare_query) {
            Some(3)
        } else {
            None
        };
        if let Some(rank) = rank {
            ranked.entry(rank).or_default().push(id.clone());
        }
    }
    let Some((_rank, mut candidates)) = ranked.into_iter().next() else {
        return Err(format!("unknown project: {input}"));
    };
    candidates.sort();
    candidates.dedup();
    if candidates.len() != 1 {
        return Err(format!(
            "ambiguous project: {input}; candidates: {}",
            candidates.join(", ")
        ));
    }
    Ok(candidates.remove(0))
}

/// Canonical `YYYY-MM-DDTHH:MM:SSZ` shape check.
///
/// Lexicographic string order equals time order only for this fixed-width
/// UTC `Z` shape (len 20, digits plus `-`/`T`/`:`/`Z` in position). Offsets
/// like `+08:00` and fractional seconds fail this check on purpose: their
/// lexicographic order does not match time order, so replay skips them
/// instead of mis-ordering.
fn is_canonical_ts(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 {
        return false;
    }
    if b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return false;
    }
    for &i in &[0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !b[i].is_ascii_digit() {
            return false;
        }
    }
    true
}

/// Replay journal `events` in slice order into a [`Materialized`] snapshot.
///
/// Each event is a journal-entry object with string `op`, object `payload`,
/// and optional string `observed_utc`. Malformed events are skipped.
/// Unknown ops (e.g. `config.set`) are ignored.
///
/// * `node.upsert`: merge `payload` into `nodes[payload.id]`; `observed_utc`
///   keeps the lexicographic max (envelope and payload candidates), every
///   other field is last-win.
/// * `edge.assert`: append one [`StoredEdge`] row (`retracted: false`).
///   `valid_from` defaults to the event `observed_utc` (else `""`, which is
///   below any real timestamp); `valid_until` `null`/missing/non-string
///   means never expires. Rows with a non-canonical timestamp are skipped:
///   a non-empty non-canonical `valid_from`, any non-canonical `valid_until`
///   string, or a non-empty non-canonical `observed_utc` (see timestamp
///   assumption below). The `""` default is exempt so missing timestamps
///   keep their historical meaning instead of being dropped.
/// * `edge.retract`: set `retracted = true` on every row with the same
///   `(from, edge, to)`; rows are retained, not filtered — callers must
///   check [`StoredEdge::retracted`]. Cost is an O(E) scan per retract,
///   fine for P1 scale; the index owns performance in Chunk 3.
/// * Validity filter (skipped when `include_expired`): keep edges with
///   `valid_from <= cutoff` and (`valid_until` none or `cutoff <=
///   valid_until`), both ends inclusive, where `cutoff` is `as_of` or now
///   when `None`. Nodes are never validity-filtered.
///
/// Timestamp assumption: UTC `Z` timestamps share one fixed-width `YYYY-MM-
/// DDTHH:MM:SSZ` shape, so lexicographic string order equals time order.
/// [`is_canonical_ts`] enforces that shape during replay; offset (`+08:00`)
/// or fractional timestamps are skipped rather than mis-ordered.
///
/// `typ.parse().unwrap_or_else(|never| match never {})` is infallible by construction: [`EdgeType`] (like
/// [`NodeType`](super::adjacency::NodeType)) maps unknown names to `Custom`
/// instead of returning `Err`.
pub fn materialize(
    events: &[serde_json::Value],
    as_of: Option<&str>,
    include_expired: bool,
) -> Materialized {
    let mut nodes: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    let mut edges: Vec<StoredEdge> = Vec::new();
    for event in events {
        let Some(op) = event.get("op").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(payload) = event.get("payload").and_then(|v| v.as_object()) else {
            continue;
        };
        let env_observed = event.get("observed_utc").and_then(|v| v.as_str());
        match op {
            "node.upsert" => {
                let Some(id) = payload.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let entry = nodes
                    .entry(id.to_string())
                    .or_insert_with(|| serde_json::json!({"id": id}));
                let Some(stored) = entry.as_object_mut() else {
                    *entry = serde_json::Value::Object(payload.clone());
                    continue;
                };
                for (k, v) in payload {
                    if k == "observed_utc" {
                        let incoming = v.as_str();
                        let current = stored.get("observed_utc").and_then(|c| c.as_str());
                        match (current, incoming) {
                            (Some(cur), Some(inc)) if cur >= inc => {}
                            (_, Some(_)) => {
                                stored.insert(k.clone(), v.clone());
                            }
                            (_, None) => {
                                stored.insert(k.clone(), v.clone());
                            }
                        }
                    } else {
                        stored.insert(k.clone(), v.clone());
                    }
                }
                if let Some(env) = env_observed {
                    let keep = stored
                        .get("observed_utc")
                        .and_then(|v| v.as_str())
                        .is_some_and(|cur| cur >= env);
                    if !keep {
                        stored.insert(
                            "observed_utc".to_string(),
                            serde_json::Value::String(env.to_string()),
                        );
                    }
                }
            }
            "edge.assert" => {
                let (Some(from), Some(to), Some(typ)) = (
                    payload.get("from").and_then(|v| v.as_str()),
                    payload.get("to").and_then(|v| v.as_str()),
                    payload.get("type").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                let edge: EdgeType = typ.parse().unwrap_or_else(|never| match never {});
                let weight = payload
                    .get("weight")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32);
                let valid_from = payload
                    .get("valid_from")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| env_observed.map(str::to_string))
                    .unwrap_or_default();
                let valid_until = payload
                    .get("valid_until")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let observed_utc = payload
                    .get("observed_utc")
                    .and_then(|v| v.as_str())
                    .or(env_observed)
                    .unwrap_or_default()
                    .to_string();
                let provenance = payload
                    .get("provenance")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if !valid_from.is_empty() && !is_canonical_ts(&valid_from) {
                    continue;
                }
                if valid_until.as_ref().is_some_and(|u| !is_canonical_ts(u)) {
                    continue;
                }
                if !observed_utc.is_empty() && !is_canonical_ts(&observed_utc) {
                    continue;
                }
                edges.push(StoredEdge {
                    from: from.to_string(),
                    edge,
                    to: to.to_string(),
                    weight,
                    valid_from,
                    valid_until,
                    retracted: false,
                    observed_utc,
                    provenance,
                });
            }
            "edge.retract" => {
                let (Some(from), Some(to), Some(typ)) = (
                    payload.get("from").and_then(|v| v.as_str()),
                    payload.get("to").and_then(|v| v.as_str()),
                    payload.get("type").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                let edge: EdgeType = typ.parse().unwrap_or_else(|never| match never {});
                for row in edges.iter_mut() {
                    if row.from == from && row.to == to && row.edge == edge {
                        row.retracted = true;
                    }
                }
            }
            _ => {}
        }
    }
    if !include_expired {
        let cutoff = as_of
            .map(str::to_string)
            .unwrap_or_else(crate::journal::observed_utc_now);
        edges.retain(|e| {
            e.valid_from.as_str() <= cutoff.as_str()
                && e.valid_until
                    .as_ref()
                    .is_none_or(|u| cutoff.as_str() <= u.as_str())
        });
    }
    Materialized { nodes, edges }
}

/// URI shape, mirroring [`validate`](super::adjacency::validate): non-empty and
/// starting with `/` or containing `://`. Extracted from the binary so CLI and
/// core share one rule.
pub fn is_uri_shape(s: &str) -> bool {
    !s.is_empty() && (s.starts_with('/') || s.contains("://"))
}

//! Typed graph nodes/edges + adjacency validation (Task 5, spec §3).
//!
//! [`NodeType::from_str`] never fails: unknown names become
//! [`NodeType::Custom`], so migrated data with extra node kinds keeps
//! loading. Same for [`EdgeType::from_str`].
//!
//! [`validate`] order: provenance check first ([`AdjacencyError::MissingProvenance`),
//! then URI rules ([`AdjacencyError::IllegalPair`] for a URI under any edge
//! other than `LOCATED_AT`/`ORIGINATED_AT`, [`AdjacencyError::BadUri`] for a
//! malformed URI), then the 43-row adjacency table. Any `Custom` party with
//! valid provenance bypasses the table (open world); core triples must match
//! a table row literally.

use std::convert::Infallible;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// Graph node kind. Unknown names parse to [`NodeType::Custom`], never `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeType {
    Conversation,
    Task,
    Decision,
    Experiment,
    Dataset,
    Model,
    Artifact,
    Project,
    Product,
    /// Open-world escape hatch. `FromStr` canonicalizes known names to core
    /// variants, so `Custom("task")` displays as `"task"` but parses back to
    /// `NodeType::Task`; only truly unknown names stay `Custom`.
    Custom(String),
}

impl NodeType {
    /// Canonical display name; a custom node round-trips its stored string.
    pub fn name(&self) -> &str {
        match self {
            NodeType::Conversation => "Conversation",
            NodeType::Task => "Task",
            NodeType::Decision => "Decision",
            NodeType::Experiment => "Experiment",
            NodeType::Dataset => "Dataset",
            NodeType::Model => "Model",
            NodeType::Artifact => "Artifact",
            NodeType::Project => "Project",
            NodeType::Product => "Product",
            NodeType::Custom(s) => s,
        }
    }
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for NodeType {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "conversation" => NodeType::Conversation,
            "task" => NodeType::Task,
            "decision" => NodeType::Decision,
            "experiment" => NodeType::Experiment,
            "dataset" => NodeType::Dataset,
            "model" => NodeType::Model,
            "artifact" => NodeType::Artifact,
            "project" => NodeType::Project,
            "product" => NodeType::Product,
            _ => NodeType::Custom(s.to_string()),
        })
    }
}

/// Graph edge kind. Unknown names parse to [`EdgeType::Custom`], never `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeType {
    BelongsTo,
    FollowsUp,
    Supports,
    Evaluates,
    DiscussedIn,
    DerivedFrom,
    Supersedes,
    LocatedAt,
    OriginatedAt,
    Contains,
    PartOf,
    Informs,
    /// Open-world escape hatch. `FromStr` canonicalizes known names to core
    /// variants, so `Custom("BELONGS_TO")` displays as `"BELONGS_TO"` but
    /// parses back to `EdgeType::BelongsTo`; only truly unknown names stay
    /// `Custom`.
    Custom(String),
}

impl EdgeType {
    /// Canonical `SCREAMING_SNAKE_CASE` name; a custom edge round-trips its string.
    pub fn name(&self) -> &str {
        match self {
            EdgeType::BelongsTo => "BELONGS_TO",
            EdgeType::FollowsUp => "FOLLOWS_UP",
            EdgeType::Supports => "SUPPORTS",
            EdgeType::Evaluates => "EVALUATES",
            EdgeType::DiscussedIn => "DISCUSSED_IN",
            EdgeType::DerivedFrom => "DERIVED_FROM",
            EdgeType::Supersedes => "SUPERSEDES",
            EdgeType::LocatedAt => "LOCATED_AT",
            EdgeType::OriginatedAt => "ORIGINATED_AT",
            EdgeType::Contains => "CONTAINS",
            EdgeType::PartOf => "PART_OF",
            EdgeType::Informs => "INFORMS",
            EdgeType::Custom(s) => s,
        }
    }
}

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for EdgeType {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_uppercase().as_str() {
            "BELONGS_TO" => EdgeType::BelongsTo,
            "FOLLOWS_UP" => EdgeType::FollowsUp,
            "SUPPORTS" => EdgeType::Supports,
            "EVALUATES" => EdgeType::Evaluates,
            "DISCUSSED_IN" => EdgeType::DiscussedIn,
            "DERIVED_FROM" => EdgeType::DerivedFrom,
            "SUPERSEDES" => EdgeType::Supersedes,
            "LOCATED_AT" => EdgeType::LocatedAt,
            "ORIGINATED_AT" => EdgeType::OriginatedAt,
            "CONTAINS" => EdgeType::Contains,
            "PART_OF" => EdgeType::PartOf,
            "INFORMS" => EdgeType::Informs,
            _ => EdgeType::Custom(s.to_string()),
        })
    }
}

/// Edge target: either a typed node or an external URI string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Node(NodeType),
    Uri(String),
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Node(n) => write!(f, "{n}"),
            Endpoint::Uri(u) => f.write_str(u),
        }
    }
}

/// Why [`validate`] rejected a triple.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdjacencyError {
    #[error("illegal adjacency: {from} -[{edge}]-> {to}")]
    IllegalPair {
        from: String,
        edge: String,
        to: String,
    },
    #[error("malformed uri: {0:?}")]
    BadUri(String),
    #[error("custom node/edge requires non-empty provenance")]
    MissingProvenance,
}

fn has_valid_provenance(provenance: Option<&str>) -> bool {
    provenance.is_some_and(|p| !p.trim().is_empty())
}

fn illegal(from: &NodeType, edge: &EdgeType, to: &Endpoint) -> AdjacencyError {
    AdjacencyError::IllegalPair {
        from: from.to_string(),
        edge: edge.to_string(),
        to: to.to_string(),
    }
}

/// Validate one `(from, edge, to)` triple.
///
/// * Any `Custom` node/edge needs `provenance = Some(non-empty)`, else
///   [`AdjacencyError::MissingProvenance`].
/// * A `Uri` endpoint is legal only under `LOCATED_AT`/`ORIGINATED_AT`
///   (else [`AdjacencyError::IllegalPair`]) and must be non-empty and start
///   with `/` or contain `://` (else [`AdjacencyError::BadUri`]).
/// * Otherwise the triple must match one of the 43 adjacency rows below.
pub fn validate(
    from: &NodeType,
    edge: &EdgeType,
    to: &Endpoint,
    provenance: Option<&str>,
) -> Result<(), AdjacencyError> {
    let custom_party = matches!(from, NodeType::Custom(_))
        || matches!(edge, EdgeType::Custom(_))
        || matches!(to, Endpoint::Node(NodeType::Custom(_)));
    if custom_party && !has_valid_provenance(provenance) {
        return Err(AdjacencyError::MissingProvenance);
    }

    if let Endpoint::Uri(uri) = to {
        if !matches!(edge, EdgeType::LocatedAt | EdgeType::OriginatedAt) {
            return Err(illegal(from, edge, to));
        }
        if uri.is_empty() || !(uri.starts_with('/') || uri.contains("://")) {
            return Err(AdjacencyError::BadUri(uri.clone()));
        }
        if custom_party {
            return Ok(());
        }
        let ok = matches!(
            (from, edge),
            (NodeType::Dataset, EdgeType::LocatedAt)
                | (NodeType::Model, EdgeType::LocatedAt)
                | (NodeType::Artifact, EdgeType::LocatedAt)
                | (NodeType::Dataset, EdgeType::OriginatedAt)
                | (NodeType::Model, EdgeType::OriginatedAt)
                | (NodeType::Artifact, EdgeType::OriginatedAt)
        );
        return if ok {
            Ok(())
        } else {
            Err(illegal(from, edge, to))
        };
    }

    if custom_party {
        return Ok(());
    }

    let Endpoint::Node(to_node) = to else {
        unreachable!("uri endpoints handled above")
    };
    let ok = matches!(
        (from, edge, to_node),
        // BELONGS_TO (5)
        (NodeType::Conversation, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Task, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Decision, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Experiment, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Artifact, EdgeType::BelongsTo, NodeType::Project)
            // FOLLOWS_UP (8)
            | (
                NodeType::Conversation,
                EdgeType::FollowsUp,
                NodeType::Conversation,
            )
            | (NodeType::Conversation, EdgeType::FollowsUp, NodeType::Task)
            | (NodeType::Task, EdgeType::FollowsUp, NodeType::Task)
            | (NodeType::Task, EdgeType::FollowsUp, NodeType::Decision)
            | (
                NodeType::Task,
                EdgeType::FollowsUp,
                NodeType::Conversation,
            )
            | (NodeType::Task, EdgeType::FollowsUp, NodeType::Artifact)
            | (NodeType::Decision, EdgeType::FollowsUp, NodeType::Task)
            | (
                NodeType::Decision,
                EdgeType::FollowsUp,
                NodeType::Decision,
            )
            // SUPPORTS (4)
            | (NodeType::Decision, EdgeType::Supports, NodeType::Task)
            | (
                NodeType::Decision,
                EdgeType::Supports,
                NodeType::Decision,
            )
            | (NodeType::Experiment, EdgeType::Supports, NodeType::Task)
            | (
                NodeType::Experiment,
                EdgeType::Supports,
                NodeType::Decision,
            )
            // EVALUATES (3)
            | (
                NodeType::Experiment,
                EdgeType::Evaluates,
                NodeType::Product,
            )
            | (
                NodeType::Experiment,
                EdgeType::Evaluates,
                NodeType::Dataset,
            )
            | (NodeType::Experiment, EdgeType::Evaluates, NodeType::Model)
            // DISCUSSED_IN (2)
            | (
                NodeType::Experiment,
                EdgeType::DiscussedIn,
                NodeType::Conversation,
            )
            | (
                NodeType::Decision,
                EdgeType::DiscussedIn,
                NodeType::Conversation,
            )
            // DERIVED_FROM (9)
            | (NodeType::Model, EdgeType::DerivedFrom, NodeType::Dataset)
            | (NodeType::Model, EdgeType::DerivedFrom, NodeType::Model)
            | (
                NodeType::Model,
                EdgeType::DerivedFrom,
                NodeType::Conversation,
            )
            | (
                NodeType::Dataset,
                EdgeType::DerivedFrom,
                NodeType::Dataset,
            )
            | (NodeType::Dataset, EdgeType::DerivedFrom, NodeType::Model)
            | (
                NodeType::Dataset,
                EdgeType::DerivedFrom,
                NodeType::Conversation,
            )
            | (
                NodeType::Artifact,
                EdgeType::DerivedFrom,
                NodeType::Dataset,
            )
            | (NodeType::Artifact, EdgeType::DerivedFrom, NodeType::Model)
            | (
                NodeType::Artifact,
                EdgeType::DerivedFrom,
                NodeType::Conversation,
            )
            // SUPERSEDES (2)
            | (
                NodeType::Artifact,
                EdgeType::Supersedes,
                NodeType::Artifact,
            )
            | (
                NodeType::Decision,
                EdgeType::Supersedes,
                NodeType::Decision,
            )
            // CONTAINS (1) / PART_OF (1)
            | (NodeType::Artifact, EdgeType::Contains, NodeType::Artifact)
            | (NodeType::Artifact, EdgeType::PartOf, NodeType::Artifact)
            // INFORMS (2)
            | (NodeType::Product, EdgeType::Informs, NodeType::Project)
            | (NodeType::Experiment, EdgeType::Informs, NodeType::Project)
    );
    if ok {
        Ok(())
    } else {
        Err(illegal(from, edge, to))
    }
}

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
/// `typ.parse().unwrap()` is infallible by construction: [`EdgeType`] (like
/// [`NodeType`]) maps unknown names to `Custom` instead of returning `Err`.
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
                let edge: EdgeType = typ.parse().unwrap();
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
                let edge: EdgeType = typ.parse().unwrap();
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

// CLI write-path helpers, extracted from `src/main.rs` (Task 8d follow-up).
//
// Placement choice: graph write-path rules live here in `graph.rs` (not a new
// module) to reuse `NodeType`/`EdgeType`/`validate`/`materialize` without
// widening visibility; config persistence lives in `config.rs`. `src/main.rs`
// calls these thinly. Behavior is byte-identical to the former inline code —
// existing CLI golden tests are the lock and pass unchanged.

/// URI shape, mirroring [`validate`]: non-empty and starting with `/` or
/// containing `://`. Extracted from the binary so CLI and core share one rule.
pub fn is_uri_shape(s: &str) -> bool {
    !s.is_empty() && (s.starts_with('/') || s.contains("://"))
}

/// Why [`check_node_provenance`] rejected a `node.upsert` write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeProvenanceError {
    #[error("custom node kind requires non-empty --provenance")]
    MissingProvenance,
}

/// CLI `graph node` custom-provenance rule (mirrors [`validate`] open-world
/// rule for the single-node case): a [`NodeType::Custom`] kind needs
/// `provenance = Some(non-empty)`, else [`NodeProvenanceError::MissingProvenance`].
/// Error string is pinned to the former binary message.
pub fn check_node_provenance(
    kind: &NodeType,
    provenance: Option<&str>,
) -> Result<(), NodeProvenanceError> {
    if matches!(kind, NodeType::Custom(_)) && provenance.is_none_or(|p| p.trim().is_empty()) {
        return Err(NodeProvenanceError::MissingProvenance);
    }
    Ok(())
}

/// CLI `graph relate` validation: resolve endpoint kinds from current
/// materialized state, then run [`validate`] plus the dangling-node fallback.
///
/// Mirrors the former binary `cmd_graph_relate` lines byte-for-byte:
/// * Best-effort [`crate::journal::Journal::open`] + `read_all` + [`materialize`];
///   open/read failures fall through to the fallback below (journal permits
///   dangling; doctor reports).
/// * Missing nodes skip table validation; the fallback still enforces URI
///   legality (non-`LOCATED_AT`/`ORIGINATED_AT`/custom edge under a URI target
///   fails as `IllegalPair` with `from: "?"`) and the custom-party provenance
///   rule (fails as [`AdjacencyError::MissingProvenance`]).
/// * Both messages match the former `eprintln!` strings exactly when the caller
///   prints `error: {e}`.
pub fn validate_relate_request(
    root: &Path,
    from: &str,
    edge: &EdgeType,
    to: &str,
    provenance: Option<&str>,
) -> Result<(), AdjacencyError> {
    let mut from_ty_opt: Option<NodeType> = None;
    let mut to_ep_opt: Option<Endpoint> = None;
    if let Ok(journal) = crate::journal::Journal::open(root) {
        if let Ok(entries) = journal.read_all() {
            let events: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "op": e.op,
                        "payload": e.payload,
                        "observed_utc": e.observed_utc,
                    })
                })
                .collect();
            let m = materialize(&events, None, true);
            if let Some(node) = m.nodes.get(from) {
                let ty = node
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Custom");
                from_ty_opt = Some(ty.parse().unwrap());
            }
            if is_uri_shape(to) {
                to_ep_opt = Some(Endpoint::Uri(to.to_string()));
            } else if let Some(node) = m.nodes.get(to) {
                let ty = node
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Custom");
                let nt: NodeType = ty.parse().unwrap();
                to_ep_opt = Some(Endpoint::Node(nt));
            }
        }
    }
    if let (Some(from_ty), Some(to_ep)) = (from_ty_opt, to_ep_opt) {
        validate(&from_ty, edge, &to_ep, provenance)
    } else {
        let edge_is_custom = matches!(edge, EdgeType::Custom(_));
        let to_is_uri = is_uri_shape(to);
        if edge_is_custom || to_is_uri {
            if to_is_uri
                && !matches!(
                    edge,
                    EdgeType::LocatedAt | EdgeType::OriginatedAt | EdgeType::Custom(_)
                )
            {
                return Err(AdjacencyError::IllegalPair {
                    from: "?".to_string(),
                    edge: edge.to_string(),
                    to: to.to_string(),
                });
            }
            if edge_is_custom && provenance.is_none_or(|p| p.trim().is_empty()) {
                return Err(AdjacencyError::MissingProvenance);
            }
        }
        Ok(())
    }
}

/// Why a strict CLI `graph relate` preflight failed before any journal write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelateRequestError {
    #[error("cannot read graph journal {path}: {detail}")]
    JournalRead { path: String, detail: String },
    #[error("invalid graph journal {path} at line {line}: {detail}")]
    InvalidJournal {
        path: String,
        line: usize,
        detail: String,
    },
    #[error("missing graph endpoint: {role} {id:?}")]
    MissingEndpoint { role: &'static str, id: String },
    #[error(transparent)]
    Adjacency(#[from] AdjacencyError),
}

/// Strict CLI `graph relate` validation over exact node IDs.
///
/// Unlike [`validate_relate_request`], this preflight reads the journal
/// without opening it through [`crate::journal::Journal`], so corrupt input is
/// reported without quarantine or repair. Missing journals represent an empty
/// graph. Every non-empty line must deserialize as a complete journal entry.
///
/// `allow_dangling` permits missing node endpoints for intentional migrations
/// and out-of-order ingestion. It does not bypass URI legality, custom-party
/// provenance, or adjacency validation when both endpoint kinds are known.
pub fn validate_relate_request_strict(
    root: &Path,
    from: &str,
    edge: &EdgeType,
    to: &str,
    provenance: Option<&str>,
    allow_dangling: bool,
) -> Result<(), RelateRequestError> {
    let journal_path = root.join(".innen/journal.jsonl");
    let content = match std::fs::read_to_string(&journal_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(RelateRequestError::JournalRead {
                path: journal_path.display().to_string(),
                detail: error.to_string(),
            });
        }
    };
    let mut events = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: crate::journal::JournalEntry =
            serde_json::from_str(line).map_err(|error| RelateRequestError::InvalidJournal {
                path: journal_path.display().to_string(),
                line: index + 1,
                detail: error.to_string(),
            })?;
        events.push(serde_json::json!({
            "op": entry.op,
            "payload": entry.payload,
            "observed_utc": entry.observed_utc,
        }));
    }

    let graph = materialize(&events, None, true);
    let from_ty = graph.nodes.get(from).map(|node| {
        node.get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("Custom")
            .parse::<NodeType>()
            .unwrap()
    });
    let to_endpoint = if is_uri_shape(to) {
        Some(Endpoint::Uri(to.to_string()))
    } else {
        graph.nodes.get(to).map(|node| {
            let kind = node
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("Custom")
                .parse::<NodeType>()
                .unwrap();
            Endpoint::Node(kind)
        })
    };

    if from_ty.is_none() && !allow_dangling {
        return Err(RelateRequestError::MissingEndpoint {
            role: "--from",
            id: from.to_string(),
        });
    }
    if to_endpoint.is_none() && !allow_dangling {
        return Err(RelateRequestError::MissingEndpoint {
            role: "--to",
            id: to.to_string(),
        });
    }

    if let (Some(from_ty), Some(to_endpoint)) = (&from_ty, &to_endpoint) {
        return validate(from_ty, edge, to_endpoint, provenance).map_err(Into::into);
    }

    if let Some(Endpoint::Uri(uri)) = &to_endpoint {
        if !matches!(
            edge,
            EdgeType::LocatedAt | EdgeType::OriginatedAt | EdgeType::Custom(_)
        ) {
            return Err(AdjacencyError::IllegalPair {
                from: "?".to_string(),
                edge: edge.to_string(),
                to: uri.clone(),
            }
            .into());
        }
    }

    let custom_party = matches!(from_ty, Some(NodeType::Custom(_)))
        || matches!(edge, EdgeType::Custom(_))
        || matches!(to_endpoint, Some(Endpoint::Node(NodeType::Custom(_))));
    if custom_party && !has_valid_provenance(provenance) {
        return Err(AdjacencyError::MissingProvenance.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_string_becomes_custom() {
        assert!(matches!(
            "Nope".parse::<NodeType>(),
            Ok(NodeType::Custom(_))
        ));
    }

    #[test]
    fn custom_node_without_provenance_rejected() {
        let err = validate(
            &NodeType::Custom("Wiki".to_string()),
            &EdgeType::BelongsTo,
            &Endpoint::Node(NodeType::Project),
            None,
        )
        .unwrap_err();
        assert_eq!(err, AdjacencyError::MissingProvenance);
    }

    #[test]
    fn custom_edge_without_provenance_rejected() {
        let err = validate(
            &NodeType::Task,
            &EdgeType::Custom("mentions".to_string()),
            &Endpoint::Node(NodeType::Decision),
            None,
        )
        .unwrap_err();
        assert_eq!(err, AdjacencyError::MissingProvenance);
    }

    #[test]
    fn adjacency_accepts_core() {
        assert!(validate(
            &NodeType::Task,
            &EdgeType::FollowsUp,
            &Endpoint::Node(NodeType::Decision),
            None,
        )
        .is_ok());
    }

    #[test]
    fn adjacency_rejects_mismatch() {
        let err = validate(
            &NodeType::Task,
            &EdgeType::Evaluates,
            &Endpoint::Node(NodeType::Project),
            None,
        )
        .unwrap_err();
        assert_eq!(
            err,
            AdjacencyError::IllegalPair {
                from: "Task".to_string(),
                edge: "EVALUATES".to_string(),
                to: "Project".to_string(),
            }
        );
    }

    #[test]
    fn bad_uri_rejected() {
        let err = validate(
            &NodeType::Dataset,
            &EdgeType::LocatedAt,
            &Endpoint::Uri(String::new()),
            None,
        )
        .unwrap_err();
        assert_eq!(err, AdjacencyError::BadUri(String::new()));
        let err = validate(
            &NodeType::Dataset,
            &EdgeType::LocatedAt,
            &Endpoint::Uri("relative/path".to_string()),
            None,
        )
        .unwrap_err();
        assert_eq!(err, AdjacencyError::BadUri("relative/path".to_string()));
    }

    #[test]
    fn uri_accepted_for_located() {
        assert!(validate(
            &NodeType::Dataset,
            &EdgeType::LocatedAt,
            &Endpoint::Uri("/x".to_string()),
            None,
        )
        .is_ok());
        assert!(validate(
            &NodeType::Dataset,
            &EdgeType::LocatedAt,
            &Endpoint::Uri("s3://b/k".to_string()),
            None,
        )
        .is_ok());
    }

    #[test]
    fn custom_collides_with_core_normalizes() {
        let custom = NodeType::Custom("task".to_string());
        let reparsed: NodeType = custom.to_string().parse().unwrap();
        assert_eq!(reparsed, NodeType::Task);
    }

    #[test]
    fn custom_bypass_accept() {
        assert!(validate(
            &NodeType::Task,
            &EdgeType::Custom("mentions".to_string()),
            &Endpoint::Node(NodeType::Decision),
            Some("test"),
        )
        .is_ok());
    }

    #[test]
    fn whitespace_provenance_rejected() {
        let err = validate(
            &NodeType::Custom("Wiki".to_string()),
            &EdgeType::BelongsTo,
            &Endpoint::Node(NodeType::Project),
            Some("   "),
        )
        .unwrap_err();
        assert_eq!(err, AdjacencyError::MissingProvenance);
    }

    #[test]
    fn case_insensitive_parse() {
        for s in ["task", "TASK", "Task"] {
            assert_eq!(s.parse::<NodeType>(), Ok(NodeType::Task));
        }
    }

    #[test]
    fn uri_under_wrong_edge_rejected() {
        let err = validate(
            &NodeType::Task,
            &EdgeType::FollowsUp,
            &Endpoint::Uri("/x".to_string()),
            None,
        )
        .unwrap_err();
        assert_eq!(
            err,
            AdjacencyError::IllegalPair {
                from: "Task".to_string(),
                edge: "FOLLOWS_UP".to_string(),
                to: "/x".to_string(),
            }
        );
    }

    #[test]
    fn adjacency_table_full_coverage() {
        let rows: Vec<(NodeType, EdgeType, Endpoint)> = vec![
            (
                NodeType::Conversation,
                EdgeType::BelongsTo,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Conversation,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Conversation,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Task),
            ),
            (
                NodeType::Task,
                EdgeType::BelongsTo,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Task,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Task),
            ),
            (
                NodeType::Task,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Decision),
            ),
            (
                NodeType::Task,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Task,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Artifact),
            ),
            (
                NodeType::Decision,
                EdgeType::BelongsTo,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Artifact,
                EdgeType::BelongsTo,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Decision,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Task),
            ),
            (
                NodeType::Decision,
                EdgeType::FollowsUp,
                Endpoint::Node(NodeType::Decision),
            ),
            (
                NodeType::Decision,
                EdgeType::Supports,
                Endpoint::Node(NodeType::Task),
            ),
            (
                NodeType::Decision,
                EdgeType::Supports,
                Endpoint::Node(NodeType::Decision),
            ),
            (
                NodeType::Experiment,
                EdgeType::BelongsTo,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Experiment,
                EdgeType::Evaluates,
                Endpoint::Node(NodeType::Product),
            ),
            (
                NodeType::Experiment,
                EdgeType::Evaluates,
                Endpoint::Node(NodeType::Dataset),
            ),
            (
                NodeType::Experiment,
                EdgeType::Evaluates,
                Endpoint::Node(NodeType::Model),
            ),
            (
                NodeType::Experiment,
                EdgeType::Supports,
                Endpoint::Node(NodeType::Task),
            ),
            (
                NodeType::Experiment,
                EdgeType::Supports,
                Endpoint::Node(NodeType::Decision),
            ),
            (
                NodeType::Experiment,
                EdgeType::DiscussedIn,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Decision,
                EdgeType::DiscussedIn,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Model,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Dataset),
            ),
            (
                NodeType::Model,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Model),
            ),
            (
                NodeType::Model,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Dataset,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Dataset),
            ),
            (
                NodeType::Dataset,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Model),
            ),
            (
                NodeType::Dataset,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Artifact,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Dataset),
            ),
            (
                NodeType::Artifact,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Model),
            ),
            (
                NodeType::Artifact,
                EdgeType::DerivedFrom,
                Endpoint::Node(NodeType::Conversation),
            ),
            (
                NodeType::Artifact,
                EdgeType::Supersedes,
                Endpoint::Node(NodeType::Artifact),
            ),
            (
                NodeType::Decision,
                EdgeType::Supersedes,
                Endpoint::Node(NodeType::Decision),
            ),
            (
                NodeType::Dataset,
                EdgeType::LocatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Model,
                EdgeType::LocatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Artifact,
                EdgeType::LocatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Dataset,
                EdgeType::OriginatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Model,
                EdgeType::OriginatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Artifact,
                EdgeType::OriginatedAt,
                Endpoint::Uri("/x".to_string()),
            ),
            (
                NodeType::Artifact,
                EdgeType::Contains,
                Endpoint::Node(NodeType::Artifact),
            ),
            (
                NodeType::Artifact,
                EdgeType::PartOf,
                Endpoint::Node(NodeType::Artifact),
            ),
            (
                NodeType::Product,
                EdgeType::Informs,
                Endpoint::Node(NodeType::Project),
            ),
            (
                NodeType::Experiment,
                EdgeType::Informs,
                Endpoint::Node(NodeType::Project),
            ),
        ];
        assert_eq!(rows.len(), 43, "plan lists exactly 43 triples");
        for (from, edge, to) in &rows {
            assert!(
                validate(from, edge, to, Some("test")).is_ok(),
                "should accept {from} -[{edge}]-> {to}"
            );
        }
        let nodes = [
            NodeType::Conversation,
            NodeType::Task,
            NodeType::Decision,
            NodeType::Experiment,
            NodeType::Dataset,
            NodeType::Model,
            NodeType::Artifact,
            NodeType::Project,
            NodeType::Product,
        ];
        let edges = [
            EdgeType::BelongsTo,
            EdgeType::FollowsUp,
            EdgeType::Supports,
            EdgeType::Evaluates,
            EdgeType::DiscussedIn,
            EdgeType::DerivedFrom,
            EdgeType::Supersedes,
            EdgeType::LocatedAt,
            EdgeType::OriginatedAt,
            EdgeType::Contains,
            EdgeType::PartOf,
            EdgeType::Informs,
        ];
        let uri = Endpoint::Uri("/x".to_string());
        let mut accepted = 0;
        for from in &nodes {
            for edge in &edges {
                for to in &nodes {
                    let to_ep = Endpoint::Node(to.clone());
                    if validate(from, edge, &to_ep, None).is_ok() {
                        accepted += 1;
                    }
                }
                if validate(from, edge, &uri, None).is_ok() {
                    accepted += 1;
                }
            }
        }
        assert_eq!(accepted, 43, "core table must accept exactly 43 triples");
    }

    #[test]
    fn materialize_applies_upserts_and_retracts() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z","label":"a"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-02T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-02T00:00:00Z","label":"b"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-03T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-03T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.retract","observed_utc":"2026-01-04T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP"}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
        assert_eq!(m.nodes.len(), 2);
        assert_eq!(m.edges.len(), 1);
        assert!(m.edges[0].retracted);
    }

    #[test]
    fn as_of_boundary_inclusive() {
        let as_of = "2026-06-01T00:00:00Z";
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-06-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:3","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-06-01T00:00:00Z","payload":{"from":"n:2","to":"n:3","type":"SUPPORTS","valid_from":"2026-06-01T00:00:00Z","valid_until":null}}),
        ];
        let m = materialize(&events, Some(as_of), false);
        assert_eq!(m.edges.len(), 2);
    }

    #[test]
    fn future_valid_from_excluded_and_null_never_expires() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-07-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z"}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
        assert_eq!(m.edges.len(), 2);
    }

    #[test]
    fn observed_utc_max_merge() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-02T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-02T00:00:00Z","label":"old"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z","label":"new"}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
        let node = m.nodes.get("n:1").expect("n:1 present");
        assert_eq!(node.get("label").and_then(|v| v.as_str()), Some("new"));
        assert_eq!(
            node.get("observed_utc").and_then(|v| v.as_str()),
            Some("2026-01-02T00:00:00Z")
        );
    }

    #[test]
    fn include_expired_bypasses_validity_filter() {
        let as_of = "2026-06-01T00:00:00Z";
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-02-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-07-01T00:00:00Z","valid_until":null}}),
        ];
        let filtered = materialize(&events, Some(as_of), false);
        assert_eq!(filtered.edges.len(), 0);
        let unfiltered = materialize(&events, Some(as_of), true);
        assert_eq!(unfiltered.edges.len(), 2);
    }

    #[test]
    fn replay_skips_malformed_and_unknown_ops() {
        let events = vec![
            serde_json::json!({"foo": 1}),
            serde_json::json!({"op": "node.upsert"}),
            serde_json::json!({"op": "node.upsert", "payload": "not-an-object"}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"label": "no-id"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from": "n:1"}}),
            serde_json::json!({"op":"config.set","observed_utc":"2026-01-01T00:00:00Z","payload":{"key": "theme", "value": "dark"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":123}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_until":null}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
        assert_eq!(m.nodes.len(), 2);
        assert_eq!(m.edges.len(), 2);
        assert!(m.edges.iter().all(|e| e.valid_until.is_none()));
        let defaulted = m.edges.iter().find(|e| e.from == "n:2").unwrap();
        assert_eq!(defaulted.valid_from, "2026-01-02T00:00:00Z");
    }

    #[test]
    fn retract_specificity() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:3","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:3","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.retract","observed_utc":"2026-01-03T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS"}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), true);
        assert_eq!(m.edges.len(), 3);
        for e in &m.edges {
            let is_target = e.from == "n:1" && e.to == "n:2" && e.edge == EdgeType::Supports;
            assert_eq!(
                e.retracted, is_target,
                "only exact (from, edge, to) retracts"
            );
        }
    }

    #[test]
    fn nodes_never_filtered() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2020-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2020-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2020-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2020-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2020-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2020-01-01T00:00:00Z","valid_until":"2020-02-01T00:00:00Z"}}),
        ];
        let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
        assert_eq!(m.edges.len(), 0);
        assert_eq!(m.nodes.len(), 2);
        assert!(m.nodes.contains_key("n:1"));
        assert!(m.nodes.contains_key("n:2"));
    }

    #[test]
    fn offset_timestamps_skipped() {
        let events = vec![
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00+08:00","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00.123Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-12-31T00:00:00+08:00"}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00+08:00","payload":{"from":"n:2","to":"n:1","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
            serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        ];
        let as_of = Some("2026-06-01T00:00:00Z");
        let filtered = materialize(&events, as_of, false);
        assert_eq!(filtered.edges.len(), 1);
        assert_eq!(filtered.edges[0].from, "n:1");
        assert_eq!(filtered.edges[0].to, "n:1");
        let unfiltered = materialize(&events, as_of, true);
        assert_eq!(unfiltered.edges.len(), 1);
    }
}

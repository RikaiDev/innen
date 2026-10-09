//! Write-path preflight for CLI `graph node` and `graph relate`.

// CLI write-path helpers, extracted from `src/main.rs`.
//
// Placement choice: graph write-path rules live in the core crate, not the
// binary, to reuse `NodeType`/`EdgeType`/`validate`/`materialize` without
// widening visibility; config persistence lives in `config.rs`. `src/main.rs`
// calls these thinly. Behavior is byte-identical to the former inline code —
// existing CLI golden tests are the lock and pass unchanged.

use super::adjacency::{
    has_valid_provenance, validate, AdjacencyError, EdgeType, Endpoint, NodeType,
};
use super::materialize::{is_uri_shape, materialize};
use std::path::Path;

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

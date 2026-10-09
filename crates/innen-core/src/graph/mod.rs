//! Typed graph nodes/edges + adjacency validation.
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

mod adjacency;
mod materialize;
mod requests;

pub use adjacency::{validate, AdjacencyError, EdgeType, Endpoint, NodeType};
pub use materialize::{is_uri_shape, materialize, resolve_project_id, Materialized, StoredEdge};
pub use requests::{
    check_node_provenance, validate_relate_request, validate_relate_request_strict,
    NodeProvenanceError, RelateRequestError,
};

#[cfg(test)]
mod tests;

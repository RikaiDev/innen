//! Typed graph nodes/edges + adjacency validation (Task 5, spec §3).
//!
//! [`NodeType::from_str`] never fails: unknown names become
//! [`NodeType::Custom`], so migrated data with extra node kinds keeps
//! loading. Same for [`EdgeType::from_str`].
//!
//! [`validate`] order: provenance check first ([`AdjacencyError::MissingProvenance`),
//! then URI rules ([`AdjacencyError::IllegalPair`] for a URI under any edge
//! other than `LOCATED_AT`/`ORIGINATED_AT`, [`AdjacencyError::BadUri`] for a
//! malformed URI), then the 42-row adjacency table. Any `Custom` party with
//! valid provenance bypasses the table (open world); core triples must match
//! a table row literally.

use std::convert::Infallible;
use std::fmt;
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
/// * Otherwise the triple must match one of the 42 adjacency rows below.
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
        // BELONGS_TO (4)
        (NodeType::Conversation, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Task, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Decision, EdgeType::BelongsTo, NodeType::Project)
            | (NodeType::Experiment, EdgeType::BelongsTo, NodeType::Project)
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
}

//! Shared `edge.assert` choke point (spec §3 write path).
//!
//! Every edge writer goes through [`append_edge_assert`] so an unknown
//! endpoint fails closed instead of landing as a dangling edge (doctor
//! `ref-integrity` class). Endpoint/adjacency/provenance semantics are
//! exactly `graph relate`'s:
//! [`crate::graph::validate_relate_request_strict`].
//!
//! `edge.retract` intentionally stays on raw
//! [`crate::journal::Journal::append`]: repairs must be able to remove
//! dangling edges.

use crate::graph::{validate_relate_request_strict, EdgeType, RelateRequestError};
use crate::journal::Journal;
use std::path::Path;

/// Why [`append_edge_assert`] refused to write.
#[derive(Debug, thiserror::Error)]
pub enum EdgeWriteError {
    #[error(transparent)]
    Request(#[from] RelateRequestError),
    #[error("journal append failed: {0}")]
    Journal(String),
}

/// One validated `edge.assert` write. Optional fields mirror the
/// `graph relate` flags; [`EdgeAssert::new`] covers the common case.
pub struct EdgeAssert<'a> {
    pub from: &'a str,
    pub edge: EdgeType,
    pub to: &'a str,
    pub weight: Option<f32>,
    pub valid_from: Option<&'a str>,
    pub valid_until: Option<&'a str>,
    pub provenance: Option<&'a str>,
    pub allow_dangling: bool,
}

impl<'a> EdgeAssert<'a> {
    pub fn new(from: &'a str, edge: EdgeType, to: &'a str) -> Self {
        Self {
            from,
            edge,
            to,
            weight: None,
            valid_from: None,
            valid_until: None,
            provenance: None,
            allow_dangling: false,
        }
    }
}

/// Strict-validate (same as `graph relate`) then append; returns the
/// convergent event id.
///
/// Validation runs against the raw journal file *before* opening it:
/// [`Journal::open`] quarantines corrupt lines and rewrites the file, so
/// opening first would let corrupt input slip through as valid. Corrupt or
/// unreadable input therefore fails without quarantine or rewrite.
pub fn append_edge_assert(root: &Path, req: &EdgeAssert<'_>) -> Result<String, EdgeWriteError> {
    validate_relate_request_strict(
        root,
        req.from,
        &req.edge,
        req.to,
        req.provenance,
        req.allow_dangling,
    )?;
    let journal = Journal::open(root).map_err(|e| EdgeWriteError::Journal(e.to_string()))?;
    let mut payload = serde_json::json!({
        "from": req.from,
        "type": req.edge.to_string(),
        "to": req.to,
    });
    if let Some(weight) = req.weight {
        payload["weight"] = serde_json::json!(weight);
    }
    if let Some(valid_from) = req.valid_from {
        payload["valid_from"] = serde_json::json!(valid_from);
    }
    if let Some(valid_until) = req.valid_until {
        payload["valid_until"] = serde_json::json!(valid_until);
    }
    if let Some(provenance) = req.provenance {
        payload["provenance"] = serde_json::json!(provenance);
    }
    journal
        .append("edge.assert", &payload)
        .map_err(|e| EdgeWriteError::Journal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upsert_node(root: &std::path::Path, id: &str, kind: &str) {
        let journal = Journal::open(root).expect("open journal");
        journal
            .append("node.upsert", &json!({"id": id, "type": kind, "label": id}))
            .expect("append node fixture");
    }

    fn journal_bytes(root: &std::path::Path) -> Vec<u8> {
        std::fs::read(root.join(".innen/journal.jsonl")).expect("read journal")
    }

    #[test]
    fn rejects_unknown_endpoint_without_writing() {
        let kb = tempfile::tempdir().expect("temp kb");
        upsert_node(kb.path(), "task:one", "Task");
        let before = journal_bytes(kb.path());
        let err = append_edge_assert(
            kb.path(),
            &EdgeAssert::new("task:one", EdgeType::BelongsTo, "project:ghost"),
        )
        .expect_err("bare-slug endpoint must fail closed");
        assert!(
            err.to_string().contains("missing graph endpoint"),
            "unexpected error: {err}"
        );
        assert_eq!(journal_bytes(kb.path()), before);
    }

    #[test]
    fn accepts_known_typed_endpoints() {
        let kb = tempfile::tempdir().expect("temp kb");
        upsert_node(kb.path(), "task:one", "Task");
        upsert_node(kb.path(), "project:one", "Project");
        let id = append_edge_assert(
            kb.path(),
            &EdgeAssert::new("task:one", EdgeType::BelongsTo, "project:one"),
        )
        .expect("known endpoints pass");
        assert!(!id.is_empty());
    }

    #[test]
    fn accepts_custom_edge_with_provenance() {
        let kb = tempfile::tempdir().expect("temp kb");
        upsert_node(kb.path(), "wiki:one", "Wiki");
        upsert_node(kb.path(), "wiki:two", "Wiki");
        let edge: EdgeType = "RELATED_TO".parse().unwrap();
        let req = EdgeAssert {
            provenance: Some("test fixture"),
            ..EdgeAssert::new("wiki:one", edge, "wiki:two")
        };
        append_edge_assert(kb.path(), &req).expect("custom edge with provenance passes");
    }

    #[test]
    fn rejects_custom_edge_without_provenance() {
        let kb = tempfile::tempdir().expect("temp kb");
        upsert_node(kb.path(), "wiki:one", "Wiki");
        upsert_node(kb.path(), "wiki:two", "Wiki");
        let edge: EdgeType = "RELATED_TO".parse().unwrap();
        let err = append_edge_assert(kb.path(), &EdgeAssert::new("wiki:one", edge, "wiki:two"))
            .expect_err("custom edge needs provenance");
        assert!(
            err.to_string().contains("provenance"),
            "unexpected error: {err}"
        );
    }
}

//! Explicit projection of canonical wiki Markdown into the append-only graph.
//!
//! The projector owns only nodes and edges marked with [`MANAGED_BY`]. It
//! preflights every Markdown page before opening the journal, never follows
//! directory symlinks, and never reads a frontmatter source reference.
use std::path::PathBuf;

use crate::journal::JournalError;
use serde::Serialize;
use thiserror::Error;

pub const MANAGED_BY: &str = "innen:wiki-sync";
pub(super) const MAX_WIKI_DEPTH: usize = 64;
pub(super) const EDGE_PROVENANCE_PREFIX: &str = "innen:wiki-sync:edge:";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub pages_scanned: u64,
    pub wiki_nodes_created: u64,
    pub wiki_nodes_updated: u64,
    pub wiki_nodes_marked_missing: u64,
    pub source_nodes_created: u64,
    pub source_nodes_updated: u64,
    pub edges_asserted: u64,
    pub edges_retracted: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum WikiGraphError {
    #[error("wiki sync I/O at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid wiki frontmatter in {path:?}: {message}")]
    Frontmatter { path: PathBuf, message: String },
    #[error("wiki graph identity conflict: {0}")]
    Identity(String),
    #[error("journal: {0}")]
    Journal(#[from] JournalError),
}

#[derive(Debug)]
pub(super) struct WikiPage {
    pub(super) path: std::path::PathBuf,
    pub(super) wiki_path: String,
    pub(super) title: String,
    pub(super) tags: Vec<String>,
    pub(super) sources: Vec<String>,
    pub(super) related: Vec<String>,
    pub(super) explicit_id: Option<String>,
    pub(super) body: String,
    pub(super) sha256: String,
    pub(super) id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct EdgeKey {
    pub(super) from: String,
    pub(super) kind: String,
    pub(super) to: String,
}

#[derive(Debug, Clone)]
pub(super) struct DesiredEdge {
    pub(super) key: EdgeKey,
    pub(super) provenance: String,
}

//! On-disk layout markers and the data model the ledger serialises.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(super) const TIMELINE: &str = "專案時間線.md";
pub(super) const INDEX: &str = ".index.json";
pub(super) const LEGACY_INDEX: &str = "檔案索引.json";
pub(super) const EVENTS: &str = ".innen-ledger/events.jsonl";
pub(super) const LOCK: &str = ".innen-ledger/ledger.lock";
pub(super) const MANIFEST: &str = ".innen-ledger/manifest.json";
pub(super) const BEGIN: &str = "<!-- innen-ledger:start -->";
pub(super) const END: &str = "<!-- innen-ledger:end -->";
pub(super) const MAGIC: &str = "innen-project-ledger-v1";
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("io: {0}")]
    Io(String),
    #[error("invalid ledger: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("downloads is intake-only: {0}")]
    Downloads(String),
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("ledger event appended but projection failed: {0}")]
    ProjectionAfterAppend(String),
}
#[derive(Debug, Clone)]
pub struct InitOptions {
    pub root: PathBuf,
    pub project_id: String,
    pub title: String,
    pub downloads_root: Option<PathBuf>,
    pub categories: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleStage {
    Intake,
    Authoring,
    Verification,
    Delivery,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Document,
    Event,
    Dataset,
    Other,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub document_id: Option<String>,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub source_kind: SourceKind,
    pub role: String,
    pub stage: LifecycleStage,
    pub owner_project: String,
    pub relation: String,
    pub observed_utc: String,
    pub document_date: Option<String>,
    pub authority: Option<String>,
    pub evidence_status: Option<String>,
    pub provenance: Option<String>,
    pub approval_valid_from: Option<String>,
    pub approval_valid_until: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    #[serde(default)]
    pub sequence: u64,
    pub kind: String,
    pub receipt_id: String,
    pub observed_utc: String,
    pub event_date: Option<String>,
    pub supersedes: Option<String>,
    pub new_path: Option<String>,
    pub expected_sha256: Option<String>,
    pub note: Option<String>,
    pub new_owner_project: Option<String>,
    pub new_relation: Option<String>,
    pub provenance: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileEvent {
    pub(super) magic: String,
    pub(super) receipt: Option<Receipt>,
    pub(super) event: Option<EventRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Projection {
    pub(super) magic: String,
    pub project_id: String,
    pub title: String,
    pub categories: Vec<String>,
    pub receipts: Vec<Receipt>,
    pub events: Vec<EventRecord>,
}
#[derive(Debug, Clone)]
pub struct AddOptions {
    pub path: PathBuf,
    pub document_id: Option<String>,
    pub source_kind: SourceKind,
    pub role: String,
    pub stage: LifecycleStage,
    pub owner_project: String,
    pub relation: String,
    pub document_date: Option<String>,
    pub authority: Option<String>,
    pub evidence_status: Option<String>,
    pub provenance: Option<String>,
    pub approval_valid_from: Option<String>,
    pub approval_valid_until: Option<String>,
    pub downloads_root: Option<PathBuf>,
}
#[derive(Debug, Clone)]
pub enum LedgerEvent {
    Note {
        receipt_id: String,
        note: String,
        event_date: Option<String>,
    },
    Supersede {
        receipt_id: String,
        superseded_receipt_id: String,
        event_date: Option<String>,
    },
    Relocate {
        receipt_id: String,
        new_path: PathBuf,
        expected_sha256: String,
        event_date: Option<String>,
    },
    Reclassify {
        receipt_id: String,
        new_owner_project: Option<String>,
        new_relation: String,
        provenance: String,
        event_date: Option<String>,
    },
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub active: usize,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    pub superseded: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Ledger {
    pub(super) root: PathBuf,
    pub(super) downloads_root: PathBuf,
    pub(super) index_path: PathBuf,
}

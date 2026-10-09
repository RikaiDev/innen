//! The retention vocabulary: what an assessment is, what can block it, and what a
//! purge or attestation returns.
//!
//! The `BLOCKER_*` strings are compared against stored values and emitted into
//! JSON reports, so they are a contract with persisted data, not free text.

use super::*;

pub(super) const SCHEMA: &str = "innen.session-retention.v1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("conversation: {0}")]
    Conversation(String),
    #[error("journal: {0}")]
    Journal(String),
    #[error("io {0}: {1}")]
    Io(String, String),
    #[error("retention proof rejected: {0}")]
    Proof(String),
    #[error("purge failed: {0}")]
    Purge(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub session_id: String,
    pub source: String,
    pub modified: Option<String>,
    pub cutoff_utc: String,
    pub source_bytes: u64,
    pub source_sha256: String,
    pub eligible: bool,
    pub blockers: Vec<String>,
    pub blocker_codes: Vec<String>,
    pub targets: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InventorySourceSummary {
    pub total: usize,
    pub eligible: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InventorySummary {
    pub schema: &'static str,
    pub total: usize,
    pub eligible: usize,
    pub eligible_bytes: u64,
    pub by_source: BTreeMap<String, InventorySourceSummary>,
    pub blocker_codes: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    pub id: String,
    pub session_id: String,
    pub source: String,
    pub source_sha256: String,
    pub knowledge_node_ids: Vec<String>,
    pub session_closed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgeReceipt {
    pub session_id: String,
    pub source: String,
    pub executed: bool,
    pub deleted_targets: Vec<PathBuf>,
    pub source_sha256: String,
    /// Other hard links to the native files, measured before deletion. When
    /// non-zero, deleting this path frees no space until those links go too.
    pub other_hard_links: u64,
    pub hard_linked_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TargetIdentity {
    pub(super) path: PathBuf,
    pub(super) device: u64,
    pub(super) inode: u64,
}

pub(super) const BLOCKER_RETENTION_WINDOW: &str = "inside_retention_window";
pub(super) const BLOCKER_MODIFIED_TIME_UNKNOWN: &str = "modified_time_unknown";
pub(super) const BLOCKER_CHILD_SESSIONS: &str = "child_sessions_present";
pub(super) const BLOCKER_ATTESTATION_MISSING: &str = "attestation_missing";
pub(super) const BLOCKER_KNOWLEDGE_MISSING: &str = "knowledge_provenance_missing";
pub(super) const BLOCKER_SESSION_ACTIVE: &str = "session_active";
pub(super) const BLOCKER_ACTIVITY_UNKNOWN: &str = "session_activity_unknown";
pub(super) const BLOCKER_ASSESSMENT_ERROR: &str = "assessment_error";
/// The native bundle is already gone. This is the desired end state, not a
/// failure: a summary row can outlive the files it describes after a purge, and
/// reporting that as an error made every later plan and sweep fail forever on
/// rows that had nothing left to clean.
pub(super) const BLOCKER_ALREADY_CLEANED: &str = "already_cleaned";

/// Hot-session window, in days, kept before an attested session may be purged.
pub const DEFAULT_RETENTION_DAYS: u64 = 7;

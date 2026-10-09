//! Read-only conversation reader. Source adapters resolve native session IDs.
//! Dialogue is a projection, not a summary: preserve corrections and chronology.

mod bounded;
mod format;
mod read;
mod record;

pub mod attachments;
pub mod checkpoint;
pub mod compact;
pub mod deltas;
pub mod grammar;
pub mod opencode_db;
pub mod pickup;
mod prefixes;
mod projection;
pub mod prune;
pub mod resume;
pub mod scan;
pub(crate) mod sources;
pub mod unfinished;
pub use checkpoint::{Checkpoint, CheckpointStatus};
pub use format::format_page;
pub use pickup::{
    resolve_all_projects_pickup, resolve_pickup, EvidencePointers, PickedUpSession, PickupTarget,
};
pub use prune::{prune_page, PruneOptions, PruneReceipt};
pub(crate) use read::stream_dialogue;
pub use read::{read, read_lines};
pub use record::{brief, Brief, EvidencePointer, Record};
pub use resume::{find_all_candidates, find_project_candidates, Candidate, ResumeTarget};
pub use scan::{ScanLimits, ScanResult};
pub use sources::{Located, Source};
pub use unfinished::{
    find_all_unfinished_candidates, find_unfinished_candidates, Confidence, Reason,
    UnfinishedCandidate,
};

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ReadError(pub(crate) String);

#[derive(Debug, Serialize)]
pub struct Page {
    pub session_id: String,
    pub source: Source,
    pub source_path: PathBuf,
    pub view: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub records: Vec<Record>,
    pub warnings: Vec<String>,
}

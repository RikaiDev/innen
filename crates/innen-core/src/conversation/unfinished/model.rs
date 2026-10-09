//! The candidate report shape: what a classification is allowed to say, and the
//! serialisation the CLI and JSON consumers depend on.

use crate::conversation::{checkpoint::Checkpoint, sources::Source};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reason {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnfinishedCandidate {
    pub id: String,
    pub source: Source,
    pub project: String,
    pub modified: Option<String>,
    pub classification: String,
    pub confidence: Confidence,
    pub reasons: Vec<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

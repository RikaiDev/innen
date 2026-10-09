//! Evidence-gated retention for native coding-tool conversations.
//!
//! innen exists to save tokens: continuing work from innen must cost less than
//! rereading a native transcript. Age alone never authorizes deletion. A session
//! becomes eligible once a proof binds its current native bytes to knowledge
//! nodes linked to the canonical conversation node: either an explicit
//! attestation, or the compact transcript `sweep` writes for a closed session
//! (user/assistant dialogue with source line numbers; tool output and
//! attachments dropped). Raw native bundles and duplicate archives are never
//! retention outputs.

use crate::conversation::resume::{find_all_candidates, Candidate};
use crate::conversation::sources::{self, Source};
use crate::graph::materialize;
use crate::ids::sha256_hex;
use crate::journal::{observed_utc_now, Journal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

mod activity;
mod archive;
mod attest;
mod compact;
mod delete_adapter;
mod inventory;
mod policy;
mod purge;
mod receipt;
mod support;
mod sweep;
mod types;

pub use attest::attest;
pub use compact::{CompactRecord, COMPACT_DIR};
pub use inventory::{inventory, summarize_inventory};
pub use purge::purge;
pub use support::cutoff_utc;
pub use sweep::{sweep, SweepItem, SweepReceipt};
pub use types::{
    Assessment, Attestation, Error, InventorySourceSummary, InventorySummary, PurgeReceipt,
    DEFAULT_RETENTION_DAYS,
};

use inventory::inventory_of;
use purge::purge_candidate;
use support::{hard_links, io, load_graph};
use types::{
    TargetIdentity, BLOCKER_ACTIVITY_UNKNOWN, BLOCKER_ALREADY_CLEANED, BLOCKER_ASSESSMENT_ERROR,
    BLOCKER_ATTESTATION_MISSING, BLOCKER_CHILD_SESSIONS, BLOCKER_KNOWLEDGE_MISSING,
    BLOCKER_MODIFIED_TIME_UNKNOWN, BLOCKER_RETENTION_WINDOW, BLOCKER_SESSION_ACTIVE, SCHEMA,
};

use activity::{session_activity, Activity};
use archive::{bundle, candidate_targets, find_candidate};
use compact::compact_and_attest;
use delete_adapter::delete_candidate;
use policy::{assess_candidate, assess_candidate_with_graph};
use receipt::{record_purge_attempt, record_purge_failure};

#[cfg(test)]
mod tests;

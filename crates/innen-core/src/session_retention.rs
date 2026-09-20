//! Evidence-gated retention for native coding-tool conversations.
//!
//! Age and a harvest watermark never authorize deletion.  A session becomes
//! eligible only after an explicit attestation binds the current native bytes
//! to live knowledge nodes linked to the canonical conversation node. Raw
//! transcripts and duplicate archives are not durable retention outputs.

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

const SCHEMA: &str = "innen.session-retention.v1";

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}

const BLOCKER_RETENTION_WINDOW: &str = "inside_retention_window";
const BLOCKER_MODIFIED_TIME_UNKNOWN: &str = "modified_time_unknown";
const BLOCKER_CHILD_SESSIONS: &str = "child_sessions_present";
const BLOCKER_ATTESTATION_MISSING: &str = "attestation_missing";
const BLOCKER_KNOWLEDGE_MISSING: &str = "knowledge_provenance_missing";
const BLOCKER_SESSION_ACTIVE: &str = "session_active";
const BLOCKER_ACTIVITY_UNKNOWN: &str = "session_activity_unknown";
const BLOCKER_ASSESSMENT_ERROR: &str = "assessment_error";

pub fn cutoff_utc(retention_days: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let cutoff = now.saturating_sub(Duration::from_secs(retention_days.saturating_mul(86_400)));
    crate::journal::format_utc(cutoff.as_secs())
}

pub fn inventory(
    root: &Path,
    source: Option<Source>,
    source_root: Option<&Path>,
    retention_days: u64,
) -> Result<Vec<Assessment>, Error> {
    let candidates = find_all_candidates(source, source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?;
    let graph = load_graph(root)?;
    candidates
        .iter()
        .map(|candidate| {
            Ok(
                assess_candidate_with_graph(&graph, candidate, retention_days).unwrap_or_else(
                    |error| Assessment {
                        session_id: candidate.id.clone(),
                        source: candidate.source.as_str().into(),
                        modified: candidate.modified.clone(),
                        cutoff_utc: cutoff_utc(retention_days),
                        source_bytes: 0,
                        source_sha256: String::new(),
                        eligible: false,
                        blockers: vec![format!("assessment failed: {error}")],
                        blocker_codes: vec![BLOCKER_ASSESSMENT_ERROR.into()],
                        targets: candidate_targets(candidate),
                    },
                ),
            )
        })
        .collect()
}

pub fn summarize_inventory(assessments: &[Assessment]) -> InventorySummary {
    let mut summary = InventorySummary {
        schema: "innen.session-retention-summary.v1",
        total: assessments.len(),
        eligible: 0,
        eligible_bytes: 0,
        by_source: BTreeMap::new(),
        blocker_codes: BTreeMap::new(),
    };
    for assessment in assessments {
        let source = summary
            .by_source
            .entry(assessment.source.clone())
            .or_default();
        source.total += 1;
        if assessment.eligible {
            source.eligible += 1;
            summary.eligible += 1;
            summary.eligible_bytes = summary
                .eligible_bytes
                .saturating_add(assessment.source_bytes);
        }
        for code in &assessment.blocker_codes {
            *summary.blocker_codes.entry(code.clone()).or_default() += 1;
        }
    }
    summary
}

pub fn attest(
    root: &Path,
    source_name: &str,
    session_id: &str,
    source_root: Option<&Path>,
    knowledge_node_ids: &[String],
    session_closed: bool,
) -> Result<Attestation, Error> {
    if !session_closed {
        return Err(Error::Proof(
            "--session-closed is required; active or ambiguous sessions are never attestable"
                .into(),
        ));
    }
    if knowledge_node_ids.is_empty() {
        return Err(Error::Proof(
            "at least one --knowledge-node is required".into(),
        ));
    }
    let source = Source::parse(source_name).map_err(|error| Error::Proof(error.to_string()))?;
    let candidate = find_candidate(source, session_id, source_root)?;
    let bundle = bundle(&candidate)?;
    match session_activity(&bundle.targets) {
        Activity::Idle => {}
        Activity::Active => {
            return Err(Error::Proof(
                "native session has an open file handle; close the coding tool before attesting"
                    .into(),
            ))
        }
        Activity::Unknown(detail) => {
            return Err(Error::Proof(format!(
                "native session activity could not be verified: {detail}"
            )))
        }
    }
    let graph = load_graph(root)?;
    let conversation_id = format!("conversation:{}:{}", source.as_str(), session_id);
    if !graph.nodes.contains_key(&conversation_id) {
        return Err(Error::Proof(format!(
            "missing canonical conversation node {conversation_id}"
        )));
    }
    let knowledge: BTreeSet<_> = knowledge_node_ids.iter().cloned().collect();
    for id in &knowledge {
        if !graph.nodes.contains_key(id) {
            return Err(Error::Proof(format!("missing knowledge node {id}")));
        }
    }
    let linked: BTreeSet<_> = graph
        .edges
        .iter()
        .filter(|edge| {
            !edge.retracted
                && edge.to == conversation_id
                && matches!(
                    edge.edge.to_string().as_str(),
                    "DERIVED_FROM" | "DISCUSSED_IN"
                )
        })
        .map(|edge| edge.from.clone())
        .collect();
    if !knowledge.iter().all(|id| linked.contains(id)) {
        return Err(Error::Proof(
            "every knowledge node must have an active DERIVED_FROM or DISCUSSED_IN edge to the conversation"
                .into(),
        ));
    }

    let id = format!(
        "session-retention:{}:{}:{}",
        source.as_str(),
        session_id,
        &bundle.sha256[..16]
    );
    let payload = json!({
        "id": id,
        "type": "SessionRetentionReceipt",
        "label": format!("verified retention proof {}:{}", source.as_str(), session_id),
        "provenance": "innen retention attest",
        "schema": SCHEMA,
        "session_id": session_id,
        "source": source.as_str(),
        "source_sha256": bundle.sha256,
        "source_bytes": bundle.bytes,
        "knowledge_node_ids": knowledge_node_ids,
        "extraction_complete": true,
        "session_closed": true,
        "observed_utc": observed_utc_now(),
    });
    Journal::open(root)
        .map_err(|error| Error::Journal(error.to_string()))?
        .append("node.upsert", &payload)
        .map_err(|error| Error::Journal(error.to_string()))?;
    Ok(Attestation {
        id: payload["id"].as_str().unwrap_or_default().to_owned(),
        session_id: session_id.to_owned(),
        source: source.as_str().to_owned(),
        source_sha256: payload["source_sha256"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        knowledge_node_ids: knowledge_node_ids.to_vec(),
        session_closed: true,
    })
}

pub fn purge(
    root: &Path,
    source_name: &str,
    session_id: &str,
    source_root: Option<&Path>,
    retention_days: u64,
    execute: bool,
) -> Result<PurgeReceipt, Error> {
    let source = Source::parse(source_name).map_err(|error| Error::Proof(error.to_string()))?;
    let candidate = find_candidate(source, session_id, source_root)?;
    let attempt_id = if execute {
        Some(record_purge_attempt(root, &candidate, retention_days)?)
    } else {
        None
    };
    let assessment = match assess_candidate(root, &candidate, retention_days) {
        Ok(assessment) => assessment,
        Err(error) => {
            if let Some(id) = attempt_id.as_deref() {
                record_purge_failure(root, id, &candidate, &error.to_string())?;
            }
            return Err(error);
        }
    };
    if !assessment.eligible {
        let error = Error::Proof(assessment.blockers.join("; "));
        if let Some(id) = attempt_id.as_deref() {
            record_purge_failure(root, id, &candidate, &error.to_string())?;
        }
        return Err(error);
    }
    if !execute {
        return Ok(PurgeReceipt {
            session_id: session_id.into(),
            source: source.as_str().into(),
            executed: false,
            deleted_targets: vec![],
            source_sha256: assessment.source_sha256,
        });
    }

    let deleted_targets = match delete_candidate(&candidate, &assessment) {
        Ok(targets) => targets,
        Err(error) => {
            if let Some(id) = attempt_id.as_deref() {
                record_purge_failure(root, id, &candidate, &error.to_string())?;
            }
            return Err(error);
        }
    };
    let receipt = json!({
        "id": format!("session-purge:{}:{}:{}", source.as_str(), session_id, &assessment.source_sha256[..16]),
        "type": "SessionPurgeReceipt",
        "label": format!("purged {}:{}", source.as_str(), session_id),
        "provenance": "innen retention purge --execute",
        "schema": SCHEMA,
        "session_id": session_id,
        "source": source.as_str(),
        "source_sha256": assessment.source_sha256,
        "attempt_id": attempt_id,
        "deleted_targets": deleted_targets,
        "observed_utc": observed_utc_now(),
    });
    Journal::open(root)
        .map_err(|error| Error::Journal(error.to_string()))?
        .append("node.upsert", &receipt)
        .map_err(|error| Error::Journal(error.to_string()))?;
    Ok(PurgeReceipt {
        session_id: session_id.into(),
        source: source.as_str().into(),
        executed: true,
        deleted_targets: serde_json::from_value(receipt["deleted_targets"].clone())
            .unwrap_or_default(),
        source_sha256: assessment.source_sha256,
    })
}

fn load_graph(root: &Path) -> Result<crate::graph::Materialized, Error> {
    let journal = Journal::open(root).map_err(|error| Error::Journal(error.to_string()))?;
    let events = journal
        .read_all()
        .map_err(|error| Error::Journal(error.to_string()))?
        .into_iter()
        .map(|entry| json!({"op": entry.op, "payload": entry.payload, "observed_utc": entry.observed_utc}))
        .collect::<Vec<_>>();
    Ok(materialize(&events, None, false))
}

fn io(path: &Path, error: impl std::fmt::Display) -> Error {
    Error::Io(path.display().to_string(), error.to_string())
}

mod activity;
mod archive;
mod delete_adapter;
mod policy;
mod receipt;
mod sweep;

pub use sweep::{sweep, SweepItem, SweepReceipt};

use activity::{session_activity, Activity};
use archive::{bundle, candidate_targets, find_candidate};
use delete_adapter::delete_candidate;
use policy::{assess_candidate, assess_candidate_with_graph};
use receipt::{record_purge_attempt, record_purge_failure};

#[cfg(test)]
#[path = "session_retention/tests.rs"]
mod tests;

//! Agent pickup command logic.
//! Selects only when unambiguous; otherwise returns concise candidate choices.
//! Emits compact checkpoint + evidence pointers + exact next action and resume command.
//! Never executes transcript text.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::checkpoint::{latest_checkpoint_for_session, resolve_project_root, Checkpoint};
use super::sources::{self, Source};
use super::unfinished::{
    find_all_unfinished_candidates, find_unfinished_candidates, Confidence, Reason,
    UnfinishedCandidate,
};
use super::ReadError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePointers {
    pub transcript_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_checkpoint_at: Option<String>,
    pub reasons: Vec<Reason>,
    pub confidence: Confidence,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickedUpSession {
    pub session_id: String,
    pub source: Source,
    pub project: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    pub evidence_pointers: EvidencePointers,
    pub next_action: String,
    pub verification: String,
    pub resume_command: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PickupTarget {
    PickedUp(Box<PickedUpSession>),
    Ambiguous {
        project: PathBuf,
        count: usize,
        candidates: Vec<UnfinishedCandidate>,
        message: String,
    },
    NotFound {
        project: PathBuf,
        message: String,
    },
}

impl PickupTarget {
    pub fn picked_up(&self) -> Option<&PickedUpSession> {
        match self {
            Self::PickedUp(s) => Some(s.as_ref()),
            _ => None,
        }
    }
}

/// Resolves an unfinished conversation for an agent to pick up.
///
/// If `session_id` is supplied, loads that session directly.
/// If omitted, scans recent unfinished candidates:
/// - If exactly 1 candidate: returns `PickedUp`
/// - If >1 candidate: returns `Ambiguous`
/// - If 0 candidate: returns `NotFound`
pub fn resolve_pickup(
    project_dir: &Path,
    session_id: Option<&str>,
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<PickupTarget, ReadError> {
    let project_root = resolve_project_root(project_dir);

    if let Some(id) = session_id {
        let validated_id = sources::validate_id(id)?;
        let src_name = source_filter
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "auto".to_string());
        let located = sources::locate(source_root, &src_name, &validated_id)?;

        let checkpoint = latest_checkpoint_for_session(&project_root, &validated_id)?;
        let (reasons, confidence, count) =
            inspect_transcript_evidence(&located.path, located.source, checkpoint.as_ref());

        let next_action = checkpoint
            .as_ref()
            .map(|cp| cp.next_action.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Inspect recent context and continue objective".into());

        let verification = checkpoint
            .as_ref()
            .map(|cp| cp.verification.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Run project test suite".into());

        let resume_command = if source_filter.is_some() {
            format!(
                "innen resume {} --source {}",
                validated_id,
                located.source.as_str()
            )
        } else {
            format!("innen resume {}", validated_id)
        };

        return Ok(PickupTarget::PickedUp(Box::new(PickedUpSession {
            session_id: validated_id,
            source: located.source,
            project: project_root,
            checkpoint,
            evidence_pointers: EvidencePointers {
                transcript_path: located.path,
                latest_checkpoint_at: None,
                reasons,
                confidence,
                event_count: count,
            },
            next_action,
            verification,
            resume_command,
        })));
    }

    let candidates = find_unfinished_candidates(&project_root, source_filter, source_root, since)?;

    match candidates.len() {
        0 => Ok(PickupTarget::NotFound {
            project: project_root.clone(),
            message: format!(
                "no unfinished candidate sessions found for project {}; specify session ID with: innen pickup <session-id>",
                project_root.display()
            ),
        }),
        1 => {
            let candidate = candidates.into_iter().next().unwrap();
            let count = count_file_lines(&candidate.path);
            let next_action = candidate
                .checkpoint
                .as_ref()
                .map(|cp| cp.next_action.clone())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Inspect recent context with resume command".into());

            let verification = candidate
                .checkpoint
                .as_ref()
                .map(|cp| cp.verification.clone())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Run project test suite".into());

            let resume_command = format!("innen resume {}", candidate.id);

            Ok(PickupTarget::PickedUp(Box::new(PickedUpSession {
                session_id: candidate.id,
                source: candidate.source,
                project: project_root,
                checkpoint: candidate.checkpoint,
                evidence_pointers: EvidencePointers {
                    transcript_path: candidate.path,
                    latest_checkpoint_at: None,
                    reasons: candidate.reasons,
                    confidence: candidate.confidence,
                    event_count: count,
                },
                next_action,
                verification,
                resume_command,
            })))
        }
        count => Ok(PickupTarget::Ambiguous {
            project: project_root.clone(),
            count,
            candidates,
            message: format!(
                "multiple candidate sessions found for project {}; pick up explicit session with: innen pickup <session-id>",
                project_root.display()
            ),
        }),
    }
}

/// Resolves an unfinished conversation across all projects for an agent to pick up.
pub fn resolve_all_projects_pickup(
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<PickupTarget, ReadError> {
    let candidates = find_all_unfinished_candidates(source_filter, source_root, since)?;

    match candidates.len() {
        0 => Ok(PickupTarget::NotFound {
            project: PathBuf::from("all-projects"),
            message: "no unfinished candidate sessions found across any projects; specify session ID with: innen pickup <session-id>".into(),
        }),
        1 => {
            let candidate = candidates.into_iter().next().unwrap();
            let count = count_file_lines(&candidate.path);
            let next_action = candidate
                .checkpoint
                .as_ref()
                .map(|cp| cp.next_action.clone())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Inspect recent context with resume command".into());

            let verification = candidate
                .checkpoint
                .as_ref()
                .map(|cp| cp.verification.clone())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Run project test suite".into());

            let proj_path = if candidate.project != "unknown" {
                PathBuf::from(&candidate.project)
            } else {
                PathBuf::from(".")
            };

            let resume_command = format!("innen resume {}", candidate.id);

            Ok(PickupTarget::PickedUp(Box::new(PickedUpSession {
                session_id: candidate.id,
                source: candidate.source,
                project: proj_path,
                checkpoint: candidate.checkpoint,
                evidence_pointers: EvidencePointers {
                    transcript_path: candidate.path,
                    latest_checkpoint_at: None,
                    reasons: candidate.reasons,
                    confidence: candidate.confidence,
                    event_count: count,
                },
                next_action,
                verification,
                resume_command,
            })))
        }
        count => Ok(PickupTarget::Ambiguous {
            project: PathBuf::from("all-projects"),
            count,
            candidates,
            message: format!(
                "multiple ({count}) unfinished candidate sessions found across projects; specify session ID with: innen pickup <session-id>"
            ),
        }),
    }
}

fn count_file_lines(path: &Path) -> usize {
    if let Ok(file) = File::open(path) {
        BufReader::new(file).lines().count()
    } else {
        0
    }
}

fn inspect_transcript_evidence(
    path: &Path,
    _source: Source,
    checkpoint: Option<&Checkpoint>,
) -> (Vec<Reason>, Confidence, usize) {
    let mut reasons = Vec::new();
    let mut count = 0;

    if let Some(cp) = checkpoint {
        reasons.push(Reason {
            code: format!("checkpoint_{}", cp.status.as_str()),
            detail: format!(
                "explicit {} checkpoint updated at {}",
                cp.status.as_str(),
                cp.updated_at
            ),
        });
    }

    if let Ok(file) = File::open(path) {
        let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        count = lines.len();
    }

    if reasons.is_empty() {
        reasons.push(Reason {
            code: "explicit_session_selection".into(),
            detail: "session explicitly requested for continuation".into(),
        });
    }

    (reasons, Confidence::High, count)
}

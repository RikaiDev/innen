//! Candidate discovery: filter resumed sessions by time window, drop the ones
//! an explicit terminal checkpoint already closed, and keep the rest as
//! candidates -- never as a semantic certainty.

use super::evidence::inspect_structural_evidence;
use super::model::UnfinishedCandidate;
use super::since::{parse_since, rfc3339_to_secs};
use crate::conversation::checkpoint::{latest_checkpoint_for_session, CheckpointStatus};
use crate::conversation::resume::{find_project_candidates, system_time_to_rfc3339};
use crate::conversation::sources::Source;
use crate::conversation::ReadError;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Discovers candidate unfinished conversations across all projects (portfolio discovery).
pub fn find_all_unfinished_candidates(
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let since_secs = parse_since(since)?;
    let base_candidates =
        crate::conversation::resume::find_all_candidates(source_filter, source_root)?;

    filter_and_analyze_candidates(base_candidates, since_secs, None)
}

/// Discovers candidate unfinished conversations for a project directory.
pub fn find_unfinished_candidates(
    project_dir: &Path,
    source_filter: Option<Source>,
    source_root: Option<&Path>,
    since: Option<&str>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let since_secs = parse_since(since)?;
    let base_candidates = find_project_candidates(project_dir, source_filter, source_root)?;

    filter_and_analyze_candidates(base_candidates, since_secs, Some(project_dir))
}

fn filter_and_analyze_candidates(
    base_candidates: Vec<crate::conversation::resume::Candidate>,
    since_secs: u64,
    scoped_project: Option<&Path>,
) -> Result<Vec<UnfinishedCandidate>, ReadError> {
    let mut unfinished = Vec::new();
    for candidate in base_candidates {
        // 1. Check timestamp against since_secs
        let mod_secs = candidate
            .modified
            .as_deref()
            .and_then(rfc3339_to_secs)
            .unwrap_or_else(|| {
                std::fs::metadata(&candidate.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });

        if mod_secs < since_secs {
            continue;
        }

        let project_str = if let Some(sp) = scoped_project {
            sp.to_string_lossy().to_string()
        } else {
            candidate
                .project
                .clone()
                .unwrap_or_else(|| "unknown".into())
        };

        // 2. Check explicit checkpoint (per-project checkpoints preserved)
        let check_dir: Option<PathBuf> = if let Some(sp) = scoped_project {
            Some(sp.to_path_buf())
        } else if project_str != "unknown" {
            let p = PathBuf::from(&project_str);
            if p.is_dir() {
                Some(p)
            } else {
                None
            }
        } else {
            None
        };

        let latest_cp = if let Some(ref dir) = check_dir {
            latest_checkpoint_for_session(dir, &candidate.id)?
        } else {
            None
        };

        if let Some(ref cp) = latest_cp {
            if matches!(
                cp.status,
                CheckpointStatus::Completed | CheckpointStatus::Superseded
            ) {
                // Explicitly finished or superseded: exclude
                continue;
            }
        }

        // 3. Inspect structural evidence
        let (reasons, confidence) =
            inspect_structural_evidence(&candidate.path, candidate.source, latest_cp.as_ref());

        unfinished.push(UnfinishedCandidate {
            id: candidate.id,
            source: candidate.source,
            project: project_str,
            modified: candidate.modified.or_else(|| {
                std::fs::metadata(&candidate.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(system_time_to_rfc3339)
            }),
            classification: "candidate".to_string(),
            confidence,
            reasons,
            checkpoint: latest_cp,
            path: candidate.path,
            parent_id: candidate.parent_id,
        });
    }

    // Sort by modified descending (newest first)
    unfinished.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(unfinished)
}

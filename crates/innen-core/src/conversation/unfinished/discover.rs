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

/// Store-level scan coverage: a silent zero must not masquerade as an
/// empty scan, so every known default store reports its state.
#[derive(Clone, serde::Serialize)]
pub struct StoreCoverage {
    pub source: String,
    pub path: String,
    pub status: String,
}

/// Reports each store from the default home layout as:
/// `scanned` (wired scanner, root on disk), `missing` (wired scanner, root
/// absent on this machine), `skipped` (excluded by the --source filter), or
/// `not-supported` (known store whose scanner is not wired in this
/// release). Coverage speaks only to whether a store was inspected; it
/// never assigns candidates to a project.
pub fn store_coverage_report(filter: Option<Source>) -> Vec<StoreCoverage> {
    let Ok(stores) = crate::conversation::sources::default_stores() else {
        return Vec::new();
    };
    stores
        .into_iter()
        .map(|(source, root)| {
            let status = if !crate::conversation::resume::scanner_wired(source) {
                "not-supported"
            } else if filter.is_some_and(|f| f != source) {
                "skipped"
            } else if !root.try_exists().unwrap_or(false) {
                "missing"
            } else {
                "scanned"
            };
            StoreCoverage {
                source: source.as_str().to_string(),
                path: root.display().to_string(),
                status: status.to_string(),
            }
        })
        .collect()
}

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
        let task_state = crate::conversation::task_state::extract_file(&candidate.path).state;
        let (reasons, confidence) = inspect_structural_evidence(
            &candidate.path,
            candidate.source,
            latest_cp.as_ref(),
            task_state.as_ref(),
            Some(&candidate.id),
        );

        // 4. A turn-level completion record (`explicit_turn_completed`)
        // stays in the report as a marker: it states the turn ended, never
        // that the task is done. Only an explicit terminal checkpoint
        // excluded a candidate above.

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
            task_state,
            path: candidate.path,
            parent_id: candidate.parent_id,
        });
    }

    // Sort by modified descending (newest first)
    unfinished.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(unfinished)
}

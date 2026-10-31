use super::activity::{
    liveness_targets, session_activity, snapshot_target_identities, verify_target_identities,
    Activity,
};
use super::*;

/// Sources this module can delete.
///
/// The command line derives its `--source` values from this list, so a source
/// with a working adapter is always reachable from the command line.
pub(super) const DELETE_ADAPTER_SOURCES: &[Source] = &[
    Source::Antigravity,
    Source::Codex,
    Source::Claude,
    Source::Copilot,
    Source::Grok,
    Source::Mcode,
    Source::Opencode,
    Source::Qwen,
];

/// Sources this module refuses to delete because no adapter exists for them.
///
/// Every `Source` variant is either listed here or in `DELETE_ADAPTER_SOURCES`;
/// `every_source_variant_is_classified` keeps that exhaustive.
pub(super) const NO_DELETE_ADAPTER_SOURCES: &[Source] =
    &[Source::Cursor, Source::Gemini, Source::Vscode];

/// Every source name the retention pipeline accepts, aliases included.
///
/// The names come from the sources that own a delete adapter, so the command
/// line never offers a source the pipeline would refuse to reclaim.
pub fn retention_source_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = DELETE_ADAPTER_SOURCES
        .iter()
        .map(|source| source.as_str())
        .collect();
    names.push("agy");
    names.push("minimax");
    names
}

pub(super) fn delete_candidate(
    candidate: &Candidate,
    assessment: &Assessment,
    source_root: Option<&Path>,
) -> Result<Vec<PathBuf>, Error> {
    // Codex and OpenCode delete through their own CLI, which only knows its
    // canonical store: with a --source-root override it would delete the
    // same id from the real store while the assessed copy stays in place.
    if source_root.is_some() && matches!(candidate.source, Source::Codex | Source::Opencode) {
        return Err(Error::Purge(format!(
            "{} deletes through its own CLI and its canonical store; refusing with --source-root",
            candidate.source.as_str()
        )));
    }
    let identities = snapshot_target_identities(&assessment.targets)?;
    let current = bundle(candidate)?;
    if current.sha256 != assessment.source_sha256 || current.bytes != assessment.source_bytes {
        return Err(Error::Purge(
            "source changed after eligibility check".into(),
        ));
    }
    verify_target_identities(&identities)?;
    match session_activity(&liveness_targets(candidate.source, &assessment.targets)) {
        Activity::Idle => {}
        Activity::Active => {
            return Err(Error::Purge(
                "native session became active after eligibility check".into(),
            ))
        }
        Activity::Unknown(detail) => {
            return Err(Error::Purge(format!(
                "native session activity could not be verified: {detail}"
            )))
        }
    }
    verify_target_identities(&identities)?;

    match candidate.source {
        Source::Codex => run_delete("codex", &["delete", "--force", &candidate.id])?,
        // --standalone keeps the delete off the shared background service: that
        // service holds the session database open for the whole host, so a
        // delete that goes through it leaves the store locked and every
        // later session reads as active.
        Source::Opencode => {
            let args = opencode_delete_args(&candidate.id);
            let borrowed = args.iter().map(String::as_str).collect::<Vec<&str>>();
            run_delete("opencode", &borrowed)?
        }
        Source::Claude | Source::Qwen => {
            fs::remove_file(&candidate.path).map_err(|error| io(&candidate.path, error))?;
        }
        Source::Antigravity => delete_antigravity(candidate, assessment)?,
        Source::Mcode | Source::Copilot | Source::Grok => {
            delete_indexed_session(candidate, assessment)?
        }
        source @ (Source::Cursor | Source::Gemini | Source::Vscode) => {
            debug_assert!(
                NO_DELETE_ADAPTER_SOURCES.contains(&source),
                "a source without a delete adapter must stay classified"
            );
            return Err(Error::Purge(format!(
                "source has no supported delete adapter: {}",
                source.as_str()
            )));
        }
    }
    // A receipt may only claim what actually happened on disk.
    let remaining: Vec<_> = if candidate.source == Source::Opencode {
        Vec::new() // the database file is shared; the CLI removes the rows
    } else {
        assessment
            .targets
            .iter()
            .filter(|target| target.exists())
            .collect()
    };
    if !remaining.is_empty() {
        return Err(Error::Purge(format!(
            "native targets still exist after deletion: {}",
            remaining
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(assessment.targets.clone())
}

/// The Antigravity store that owns a session's native bytes.
///
/// Antigravity ships three store layouts on one machine — `antigravity-cli`,
/// `antigravity` and `antigravity-ide` — and a session belongs to whichever
/// one holds it.
pub(super) fn antigravity_store_root(targets: &[PathBuf]) -> Option<PathBuf> {
    targets
        .iter()
        .find_map(|target| {
            target.ancestors().find(|ancestor| {
                ancestor
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        matches!(name, "antigravity-cli" | "antigravity" | "antigravity-ide")
                    })
            })
        })
        .map(Path::to_path_buf)
}

mod antigravity;
pub(super) mod sqlite;

use antigravity::delete_antigravity;
use sqlite::delete_indexed_session;

/// Arguments for `opencode session delete`.
///
/// The delete runs against a private server: the shared background service
/// keeps the session database open for the whole host, and a delete routed
/// through it leaves that handle behind.
pub(super) fn opencode_delete_args(session_id: &str) -> Vec<String> {
    vec![
        "session".to_string(),
        "delete".to_string(),
        session_id.to_string(),
        "--standalone".to_string(),
    ]
}

pub(super) fn run_delete(program: &str, args: &[&str]) -> Result<(), Error> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| Error::Purge(format!("{program}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(512)
            .collect::<String>();
        let detail = detail.trim();
        if detail.is_empty() {
            Err(Error::Purge(format!("{program} exited {}", output.status)))
        } else {
            Err(Error::Purge(format!(
                "{program} exited {}: {detail}",
                output.status
            )))
        }
    }
}

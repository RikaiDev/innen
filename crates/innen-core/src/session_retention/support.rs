//! Primitives shared by every retention module: reading the materialized graph,
//! formatting an io failure, computing the retention cutoff, and measuring hard
//! links left behind by other tools.

use super::*;

/// Hot-session window boundary: anything modified at or after this instant is
/// still inside the retention window.
pub fn cutoff_utc(retention_days: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let cutoff = now.saturating_sub(Duration::from_secs(retention_days.saturating_mul(86_400)));
    crate::journal::format_utc(cutoff.as_secs())
}

/// Largest count of *other* hard links among the native files under `targets`,
/// and the bytes of the files that have any. Another tool that hard-links a
/// native store keeps those bytes on disk after this path is deleted.
pub(super) fn hard_links(targets: &[PathBuf]) -> (u64, u64) {
    fn visit(path: &Path, links: &mut u64, bytes: &mut u64) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if metadata.is_dir() {
            for entry in fs::read_dir(path).into_iter().flatten().flatten() {
                visit(&entry.path(), links, bytes);
            }
        } else if metadata.is_file() {
            #[cfg(unix)]
            {
                let others = metadata.nlink().saturating_sub(1);
                if others > 0 {
                    *links = (*links).max(others);
                    *bytes = bytes.saturating_add(metadata.len());
                }
            }
        }
    }
    let (mut links, mut bytes) = (0, 0);
    for target in targets {
        visit(target, &mut links, &mut bytes);
    }
    (links, bytes)
}

pub(super) fn load_graph(root: &Path) -> Result<crate::graph::Materialized, Error> {
    let journal = Journal::open(root).map_err(|error| Error::Journal(error.to_string()))?;
    let events = journal
        .read_all()
        .map_err(|error| Error::Journal(error.to_string()))?
        .into_iter()
        .map(|entry| json!({"op": entry.op, "payload": entry.payload, "observed_utc": entry.observed_utc}))
        .collect::<Vec<_>>();
    Ok(materialize(&events, None, false))
}

pub(super) fn io(path: &Path, error: impl std::fmt::Display) -> Error {
    Error::Io(path.display().to_string(), error.to_string())
}

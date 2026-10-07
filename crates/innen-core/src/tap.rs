//! Tap trait + identity cursor.
//!
//! Credential scanning lives in [`crate::credentials`]: the scan runs in
//! the caller (harvest/ingest) BEFORE any append — a hit skips the file +
//! reports it, never appends.
//!
//! The cursor records *which* inputs were consumed, never how many. A count
//! is invalid over a directory whose files are deleted after ingestion: each
//! deletion shifts the window by one, and once the count exceeds the listing
//! the tap reports "nothing new" forever while unprocessed files sit in the
//! inbox. That is exactly what happened to `harvest-dir` — a count of 875
//! against 96 files, reported as an empty backlog instead of an error.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub trait Tap {
    fn id(&self) -> &str;
    fn cursor(&self) -> CursorState;
    fn collect(&self) -> Result<Vec<Event>, TapError>;
}

#[derive(Debug, Clone)]
pub struct Event {
    pub op: String,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TapError {
    #[error("io: {0}")]
    Io(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("credential hit: {pattern} in {file}")]
    CredentialHit { file: String, pattern: String },
}

/// Cursor file schema. Bumped whenever the on-disk shape changes so an older
/// file is recognised as legacy instead of silently read as empty.
pub const WATERMARK_SCHEMA: &str = "innen.tap.watermark.v2";

/// The consumed inputs of one tap. Sorted, so the file is byte-stable.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    #[serde(default)]
    pub consumed: BTreeSet<String>,
}

impl Cursor {
    /// The listed inputs this cursor has not consumed, in listing order.
    pub fn unconsumed<'a>(&self, listed: &'a [String]) -> Vec<&'a String> {
        listed.iter().filter(|name| !self.consumed.contains(*name)).collect()
    }
}

/// What the cursor file on disk actually said. The state is part of the
/// report so a rebuilt or unreadable cursor is visible to the caller instead
/// of being flattened into "nothing new".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorState {
    /// No cursor file: nothing has been consumed.
    Missing,
    /// A pre-v2 bare count. It cannot name the files it covered, so every
    /// currently listed file is treated as unconsumed. Replay is convergent —
    /// the journal event id is derived from content and the op is an upsert —
    /// but the caller must say so rather than present it as a clean backlog.
    LegacyCount(u64),
    Current(Cursor),
    /// Present but not parseable as either shape. Same conservative
    /// re-scan as legacy, reported as a fault.
    Unreadable(String),
}

impl CursorState {
    /// The consumed set, or `None` when the file cannot be trusted to name
    /// its inputs (legacy count, unreadable).
    pub fn known(&self) -> Option<&BTreeSet<String>> {
        match self {
            CursorState::Current(cursor) => Some(&cursor.consumed),
            _ => None,
        }
    }

    /// True when the cursor had to be discarded and the inbox re-scanned.
    pub fn rebuilt(&self) -> bool {
        matches!(self, CursorState::LegacyCount(_) | CursorState::Unreadable(_))
    }

    /// Stable, human-readable state name for reports.
    pub fn label(&self) -> &'static str {
        match self {
            CursorState::Missing => "missing",
            CursorState::LegacyCount(_) => "legacy_count_rebuilt",
            CursorState::Current(_) => "current",
            CursorState::Unreadable(_) => "unreadable_rebuilt",
        }
    }
}

/// `<root>/.innen/tap/{tap_id}.watermark`
///
/// NOTE: `tap_id` is joined verbatim into the path; the ids in use are
/// hardcoded constants (`harvest-dir`) so this is safe — dynamic tap ids
/// must be sanitized (known limitation, no code change).
pub fn watermark_path(root: &Path, tap_id: &str) -> PathBuf {
    root.join(".innen")
        .join("tap")
        .join(format!("{tap_id}.watermark"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredCursor {
    schema: String,
    #[serde(default)]
    consumed: BTreeSet<String>,
}

/// Read the cursor file and report what it was, without guessing.
///
/// A bare integer is the pre-v2 count format and is surfaced as
/// [`CursorState::LegacyCount`]; anything else unparseable becomes
/// [`CursorState::Unreadable`]. Neither is silently read as "nothing
/// consumed", because that is the failure this type exists to prevent.
pub fn load_cursor(root: &Path, tap_id: &str) -> CursorState {
    let path = watermark_path(root, tap_id);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CursorState::Missing,
        Err(e) => return CursorState::Unreadable(format!("read {}: {e}", path.display())),
    };
    let trimmed = text.trim();
    if let Ok(count) = trimmed.parse::<u64>() {
        return CursorState::LegacyCount(count);
    }
    match serde_json::from_str::<StoredCursor>(trimmed) {
        Ok(stored) if stored.schema == WATERMARK_SCHEMA => {
            CursorState::Current(Cursor { consumed: stored.consumed })
        }
        Ok(stored) => CursorState::Unreadable(format!(
            "unknown cursor schema {:?} in {}",
            stored.schema,
            path.display()
        )),
        Err(e) => CursorState::Unreadable(format!("{}: {e}", path.display())),
    }
}

/// Replace the cursor file with `cursor`. Unlike the old count this is not
/// monotonic: consuming fewer files is normal once files are deleted, and
/// refusing to record that would be the original bug.
pub fn store_cursor(root: &Path, tap_id: &str, cursor: &Cursor) -> Result<(), TapError> {
    let path = watermark_path(root, tap_id);
    let stored = StoredCursor {
        schema: WATERMARK_SCHEMA.to_string(),
        consumed: cursor.consumed.clone(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TapError::Io(format!("create_dir {}: {e}", parent.display())))?;
    }
    let tmp = path.with_extension("watermark.partial");
    std::fs::write(
        &tmp,
        serde_json::to_string(&stored)
            .map_err(|e| TapError::Parse(format!("serialize cursor: {e}")))?,
    )
    .map_err(|e| TapError::Io(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &path).map_err(|e| TapError::Io(format!("rename {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_cursor_is_missing_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_cursor(dir.path(), "t"), CursorState::Missing);
        assert!(!watermark_path(dir.path(), "t").exists());
    }

    #[test]
    fn legacy_count_is_named_not_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = watermark_path(dir.path(), "t");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"875").unwrap();
        let state = load_cursor(dir.path(), "t");
        assert_eq!(state, CursorState::LegacyCount(875));
        assert!(state.rebuilt());
        assert!(state.known().is_none(), "a count cannot name its inputs");
    }

    #[test]
    fn unparseable_cursor_is_reported_not_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = watermark_path(dir.path(), "t");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        let state = load_cursor(dir.path(), "t");
        assert!(matches!(state, CursorState::Unreadable(_)));
        assert!(state.rebuilt());
    }

    #[test]
    fn cursor_round_trips_and_shrinks_when_files_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut cursor = Cursor::default();
        cursor.consumed.extend(["a.md".to_string(), "b.md".to_string()]);
        store_cursor(dir.path(), "t", &cursor).unwrap();
        assert_eq!(
            load_cursor(dir.path(), "t"),
            CursorState::Current(cursor.clone())
        );
        // A later run that consumed one file must be able to record the
        // smaller set; the old count API refused exactly this.
        cursor.consumed.remove("b.md");
        store_cursor(dir.path(), "t", &cursor).unwrap();
    }
}
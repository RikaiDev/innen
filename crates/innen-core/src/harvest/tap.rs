//! The tap itself: what it watches, what it has already consumed, and the one
//! event shape a pending file becomes.

use std::path::PathBuf;

use crate::tap::{Event, Tap, TapError};

use super::scan::{event_for_file, unconsumed};

/// Pinned tap id for the harvest directory tap.
pub const TAP_ID: &str = "harvest-dir";

/// Watches `<dir>/00-inbox/harvest` where `dir` is the KB root.
#[derive(Debug, Clone)]
pub struct DirectoryTap {
    pub dir: PathBuf,
}

impl DirectoryTap {
    fn inbox_dir(&self) -> PathBuf {
        self.dir.join("00-inbox/harvest")
    }
}

impl Tap for DirectoryTap {
    fn id(&self) -> &str {
        TAP_ID
    }

    fn cursor(&self) -> crate::tap::CursorState {
        crate::tap::load_cursor(&self.dir, TAP_ID)
    }

    /// Files this tap has not consumed, one `node.upsert` event each. No
    /// credential filtering here — the caller ([`check`]/[`ingest::run`])
    /// scans before append.
    fn collect(&self) -> Result<Vec<Event>, TapError> {
        let mut out = Vec::new();
        for rel in unconsumed(&self.dir).0 {
            let path = self.inbox_dir().join(&rel);
            let bytes = std::fs::read(&path)
                .map_err(|e| TapError::Io(format!("read {}: {e}", path.display())))?;
            let text = String::from_utf8(bytes.clone())
                .map_err(|e| TapError::Parse(format!("{rel}: non-utf8 markdown: {e}")))?;
            out.push(event_for_file(&rel, &bytes, &text));
        }
        Ok(out)
    }
}

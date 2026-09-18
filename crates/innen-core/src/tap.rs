//! Tap trait + watermark.
//!
//! Credential scanning lives in [`crate::credentials`]: the scan runs in
//! the caller (harvest/ingest) BEFORE any append — a hit skips the file +
//! reports it, never appends.

use std::path::{Path, PathBuf};

pub trait Tap {
    fn id(&self) -> &str;
    fn watermark(&self) -> u64;
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

/// Missing or unparseable watermark file → 0.
pub fn load_watermark(root: &Path, tap_id: &str) -> u64 {
    let path = watermark_path(root, tap_id);
    match std::fs::read_to_string(&path) {
        Ok(text) => text.trim().parse::<u64>().unwrap_or(0),
        Err(_) => 0,
    }
}

/// Store watermark `v`, refusing to decrease the stored value.
///
/// If the file holds a parseable `cur` with `v < cur`, returns
/// `Err(TapError::Io(..))` leaving the file untouched. Missing or
/// unparseable files are treated as 0 and overwritten.
pub fn store_watermark(root: &Path, tap_id: &str, v: u64) -> Result<(), TapError> {
    let path = watermark_path(root, tap_id);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(cur) = text.trim().parse::<u64>() {
            if v < cur {
                return Err(TapError::Io(format!(
                    "refuse watermark decrease for {tap_id}: stored {cur} > new {v} ({})",
                    path.display()
                )));
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TapError::Io(format!("create_dir {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, v.to_string().as_bytes())
        .map_err(|e| TapError::Io(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watermark_missing_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_watermark(dir.path(), "t"), 0);
        assert!(!watermark_path(dir.path(), "t").exists());
    }

    #[test]
    fn watermark_refuses_decrease() {
        let dir = tempfile::tempdir().unwrap();
        store_watermark(dir.path(), "t", 5).unwrap();
        assert!(store_watermark(dir.path(), "t", 3).is_err());
        assert_eq!(load_watermark(dir.path(), "t"), 5);
    }

    #[test]
    fn watermark_unparseable_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = watermark_path(dir.path(), "t");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"junk").unwrap();
        assert_eq!(load_watermark(dir.path(), "t"), 0);
    }

}

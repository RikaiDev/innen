//! Tap trait + watermark + credential scan (Task 13).
//!
//! Credential patterns (sourced: GitHub secret-scanning docs + AWS docs).
//! False positives go to a human via report, never auto-delete.
//!
//! Regex-free subset (no new dependencies — hand-rolled matchers):
//! - `aws-access-key` (`AKIA[0-9A-Z]{16}`): fixed prefix `AKIA` + 16 chars
//!   each in `[0-9A-Z]`. Matches the regex on ASCII input; no word-boundary
//!   handling (same as the bare regex).
//! - `github-token` (`ghp_[A-Za-z0-9]{36,}`): fixed prefix `ghp_` + run of
//!   `[A-Za-z0-9]` of length ≥ 36. Only alphanumerics counted (no `_` tail).
//! - `private-key` (`-----BEGIN .*PRIVATE KEY`): line-oriented — a line
//!   containing literal `-----BEGIN ` with `PRIVATE KEY` later on the same
//!   line. Covers single-line PEM headers (`-----BEGIN RSA PRIVATE KEY-----`);
//!   multi-line split headers are not matched.
//! - `client-secret` (`client_secret\s*[:=]`): literal `client_secret`,
//!   ASCII-whitespace skip (` `, `\t`, `\n`, `\r`, VT, FF), then `:` or `=`.
//!   (Regex `\s` also covers Unicode whitespace; we cover ASCII only.)
//! - `slack-token` (`xox[bap]-`): literal `xoxb-` / `xoxa-` / `xoxp-`.
//! - `anthropic-key` (`sk-ant-[A-Za-z0-9-]+`): fixed prefix `sk-ant-` +
//!   ≥ 1 chars each in `[A-Za-z0-9-]`. No minimum-length beyond 1.

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
/// NOTE: `tap_id` is joined verbatim into the path; P2 ids are hardcoded
/// constants (`harvest-dir`) so this is safe — dynamic tap ids must be
/// sanitized (known limitation, no code change).
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

fn is_aws_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z')
}

fn is_ghp_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z')
}

fn is_anthropic_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-')
}

fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn has_aws_key(bytes: &[u8]) -> bool {
    if bytes.len() < 20 {
        return false;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"AKIA")
            && i + 20 <= bytes.len()
            && bytes[i + 4..i + 20].iter().all(|&b| is_aws_tail(b))
        {
            return true;
        }
    }
    false
}

fn has_github_token(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"ghp_") {
            let mut n = 0;
            for &b in &bytes[i + 4..] {
                if is_ghp_tail(b) {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= 36 {
                return true;
            }
        }
    }
    false
}

fn has_private_key(text: &str) -> bool {
    for line in text.lines() {
        if let Some(begin) = line.find("-----BEGIN ") {
            if line[begin + "-----BEGIN ".len()..].contains("PRIVATE KEY") {
                return true;
            }
        }
    }
    false
}

fn has_client_secret(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"client_secret";
    if bytes.len() < NEEDLE.len() {
        return false;
    }
    for i in 0..=(bytes.len() - NEEDLE.len()) {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut j = i + NEEDLE.len();
            while j < bytes.len() && is_ascii_ws(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                return true;
            }
        }
    }
    false
}

fn has_slack_token(bytes: &[u8]) -> bool {
    if bytes.len() < 5 {
        return false;
    }
    for i in 0..=(bytes.len() - 5) {
        let w = &bytes[i..i + 5];
        if w == b"xoxb-" || w == b"xoxa-" || w == b"xoxp-" {
            return true;
        }
    }
    false
}

fn has_anthropic_key(bytes: &[u8]) -> bool {
    const PREFIX: &[u8] = b"sk-ant-";
    if bytes.len() <= PREFIX.len() {
        // Need prefix + at least one tail char.
        return false;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX
            && i + PREFIX.len() < bytes.len()
            && is_anthropic_tail(bytes[i + PREFIX.len()])
        {
            return true;
        }
    }
    false
}

/// Return matched credential pattern NAMES (each at most once, canonical order).
pub fn scan_credentials(text: &str) -> Vec<&'static str> {
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    if has_aws_key(bytes) {
        hits.push("aws-access-key");
    }
    if has_github_token(bytes) {
        hits.push("github-token");
    }
    if has_private_key(text) {
        hits.push("private-key");
    }
    if has_client_secret(bytes) {
        hits.push("client-secret");
    }
    if has_slack_token(bytes) {
        hits.push("slack-token");
    }
    if has_anthropic_key(bytes) {
        hits.push("anthropic-key");
    }
    hits
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

    #[test]
    fn scan_hits_aws_key_and_private_key() {
        let hits = scan_credentials("key AKIAIOSFODNN7EXAMPLE\n-----BEGIN RSA PRIVATE KEY-----\n");
        assert!(hits.contains(&"aws-access-key"));
        assert!(hits.contains(&"private-key"));
    }

    #[test]
    fn scan_clean_text_empty() {
        assert!(scan_credentials("hello world, nothing secret here").is_empty());
    }
}

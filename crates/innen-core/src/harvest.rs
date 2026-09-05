//! DirectoryTap + harvest check + ingest (Task 14).
//!
//! Pinned inbox: `<root>/00-inbox/harvest` (flat `*.md`, sorted).
//! Mapping: `id = "source:<sha8(relative_path)>"` (sha8 = first 8 hex of
//! `sha256_hex(relative_path)`); provenance `{path, bytes, sha256,
//! observed_utc}`; `idempotency_key` = full content sha256 hex; watermark =
//! files consumed count via [`crate::tap`] (`harvest-dir`).
//!
//! Known limitation (accepted for P2): the watermark is count-based, not
//! content-addressed, so rename/delete shifts counts and misaligns the
//! consumed prefix. A content-addressed cursor is future work.
//!
//! Credential scan runs in the caller ([`check`]/[`ingest::run`]) BEFORE any
//! append: a hit skips the file + reports it, never appends. [`TapError`]
//! [`crate::tap::TapError::CredentialHit`] is for direct `Tap` API users only.
//!
//! [`DirectoryTap::dir`] is the KB root (not the inbox dir itself): the inbox
//! is `<dir>/00-inbox/harvest` and the watermark is
//! [`crate::tap::watermark_path`]`(dir, "harvest-dir")`.

use std::path::{Path, PathBuf};

use crate::tap::{load_watermark, Event, Tap, TapError};

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

    fn watermark(&self) -> u64 {
        load_watermark(&self.dir, TAP_ID)
    }

    /// Sorted `*.md` files past the watermark count, one `node.upsert`
    /// event per file. No credential filtering here — the caller
    /// ([`check`]/[`ingest::run`]) scans before append.
    fn collect(&self) -> Result<Vec<Event>, TapError> {
        let files = list_inbox_files(&self.dir);
        let skip = self.watermark() as usize;
        let mut out = Vec::new();
        for rel in files.into_iter().skip(skip) {
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

/// Dry-run report for one tap.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TapReport {
    pub id: String,
    pub new_files: Vec<String>,
    pub skipped: Vec<String>,
}

/// Dry-run report over all taps (P2: exactly one, `harvest-dir`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarvestReport {
    pub taps: Vec<TapReport>,
}

/// A file skipped for credentials (never appended).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Skipped {
    pub path: String,
    pub pattern: String,
    pub preview: String,
}

/// Result of [`ingest::run`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IngestReport {
    pub added: u64,
    pub skipped: Vec<Skipped>,
}

/// Sorted top-level `*.md` file names under `<root>/00-inbox/harvest`.
/// Missing/unreadable inbox → empty.
fn list_inbox_files(root: &Path) -> Vec<String> {
    let inbox = root.join("00-inbox/harvest");
    let entries = match std::fs::read_dir(&inbox) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|o| o.to_str())
            .is_some_and(|e| e == "md");
        if !is_md {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    names
}

fn sha8(rel: &str) -> String {
    crate::ids::sha256_hex(rel.as_bytes())[..8].to_string()
}

fn source_id(rel: &str) -> String {
    format!("source:{}", sha8(rel))
}

fn event_for_file(rel: &str, bytes: &[u8], text: &str) -> Event {
    let sha = crate::ids::sha256_hex(bytes);
    let payload = serde_json::json!({
        "id": source_id(rel),
        "type": crate::graph::NodeType::Artifact.to_string(),
        "label": rel,
        "body": text,
        "provenance": {
            "path": rel,
            "bytes": bytes.len() as u64,
            "sha256": sha,
            "observed_utc": crate::journal::observed_utc_now(),
        },
    });
    Event {
        op: "node.upsert".to_string(),
        payload,
        idempotency_key: sha,
    }
}

/// DRY-RUN: no appends, no watermark advance.
pub fn check(root: &Path) -> HarvestReport {
    let files = list_inbox_files(root);
    let wm = load_watermark(root, TAP_ID) as usize;
    let mut new_files = Vec::new();
    let mut skipped = Vec::new();
    let inbox = root.join("00-inbox/harvest");
    for rel in files.into_iter().skip(wm) {
        let text = std::fs::read_to_string(inbox.join(&rel)).unwrap_or_default();
        if crate::tap::scan_credentials(&text).is_empty() {
            new_files.push(rel);
        } else {
            skipped.push(rel);
        }
    }
    HarvestReport {
        taps: vec![TapReport {
            id: TAP_ID.to_string(),
            new_files,
            skipped,
        }],
    }
}

pub mod ingest {
    use super::*;

    /// Append one `node.upsert` per clean file, skip credential hits
    /// (reported, never appended), then advance the watermark past all
    /// consumed files (added + skipped).
    pub fn run(root: &Path) -> IngestReport {
        let files = list_inbox_files(root);
        let wm = load_watermark(root, TAP_ID) as usize;
        let pending: Vec<String> = files.into_iter().skip(wm).collect();
        let total = pending.len();
        if total == 0 {
            return IngestReport {
                added: 0,
                skipped: Vec::new(),
            };
        }
        let Ok(journal) = crate::journal::Journal::open(root) else {
            return IngestReport {
                added: 0,
                skipped: Vec::new(),
            };
        };
        let inbox = root.join("00-inbox/harvest");
        let mut added: u64 = 0;
        let mut skipped = Vec::new();
        let mut processed: usize = 0;
        for rel in &pending {
            let bytes = match std::fs::read(inbox.join(rel)) {
                Ok(b) => b,
                Err(_) => {
                    processed += 1;
                    continue;
                }
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let hits = crate::tap::scan_credentials(&text);
            if let Some(pattern) = hits.first() {
                skipped.push(Skipped {
                    path: rel.clone(),
                    pattern: pattern.to_string(),
                    preview: credential_preview(&text, pattern),
                });
                processed += 1;
                continue;
            }
            let event = event_for_file(rel, &bytes, &text);
            match journal.append(&event.op, &event.payload) {
                Ok(_) => {
                    added += 1;
                    processed += 1;
                }
                Err(_) => break,
            }
        }
        if processed > 0 {
            let _ = crate::tap::store_watermark(root, TAP_ID, wm as u64 + processed as u64);
        }
        IngestReport { added, skipped }
    }
}

/// Redacted preview: first 6 chars starting at the first match of `pattern`
/// + `"***"`. Falls back to the first 6 chars of content when the pattern
/// has no locatable match.
fn credential_preview(content: &str, pattern: &str) -> String {
    let off = match pattern {
        "aws-access-key" => find_aws_offset(content.as_bytes()),
        "github-token" => find_github_offset(content.as_bytes()),
        "private-key" => find_private_key_offset(content),
        "client-secret" => find_client_secret_offset(content.as_bytes()),
        "slack-token" => find_slack_offset(content.as_bytes()),
        "anthropic-key" => find_anthropic_offset(content.as_bytes()),
        _ => None,
    };
    match off.and_then(|o| content.get(o..)) {
        Some(tail) => format!("{}***", tail.chars().take(6).collect::<String>()),
        None => format!("{}***", content.chars().take(6).collect::<String>()),
    }
}

fn find_aws_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 20 {
        return None;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"AKIA")
            && i + 20 <= bytes.len()
            && bytes[i + 4..i + 20]
                .iter()
                .all(|&b| matches!(b, b'0'..=b'9' | b'A'..=b'Z'))
        {
            return Some(i);
        }
    }
    None
}

fn find_github_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"ghp_") {
            let mut n = 0;
            for &b in &bytes[i + 4..] {
                if matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z') {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= 36 {
                return Some(i);
            }
        }
    }
    None
}

fn find_private_key_offset(text: &str) -> Option<usize> {
    // Byte offset of the `-----BEGIN ` marker on a line that also contains
    // `PRIVATE KEY` later on the same line.
    let mut base = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some(pos) = line.find("-----BEGIN ") {
            if line[pos + "-----BEGIN ".len()..].contains("PRIVATE KEY") {
                return Some(base + pos);
            }
        }
        base += line.len();
    }
    None
}

fn find_client_secret_offset(bytes: &[u8]) -> Option<usize> {
    const NEEDLE: &[u8] = b"client_secret";
    if bytes.len() < NEEDLE.len() {
        return None;
    }
    for i in 0..=(bytes.len() - NEEDLE.len()) {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut j = i + NEEDLE.len();
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
            {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                return Some(i);
            }
        }
    }
    None
}

fn find_slack_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 5 {
        return None;
    }
    for i in 0..=(bytes.len() - 5) {
        let w = &bytes[i..i + 5];
        if w == b"xoxb-" || w == b"xoxa-" || w == b"xoxp-" {
            return Some(i);
        }
    }
    None
}

fn find_anthropic_offset(bytes: &[u8]) -> Option<usize> {
    const PREFIX: &[u8] = b"sk-ant-";
    if bytes.len() <= PREFIX.len() {
        return None;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX
            && i + PREFIX.len() < bytes.len()
            && matches!(
                bytes[i + PREFIX.len()],
                b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-'
            )
        {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tap::watermark_path;

    fn write_inbox(dir: &tempfile::TempDir, files: &[(&str, &str)]) {
        let inbox = dir.path().join("00-inbox/harvest");
        std::fs::create_dir_all(&inbox).unwrap();
        for (name, content) in files {
            std::fs::write(inbox.join(name), content).unwrap();
        }
    }

    #[test]
    fn harvest_picks_new_files_only() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "hello"), ("b.md", "world")]);
        let r1 = check(dir.path());
        assert_eq!(
            r1.taps[0].new_files,
            vec!["a.md".to_string(), "b.md".to_string()]
        );
        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 2);
        assert!(ing.skipped.is_empty());
        let r2 = check(dir.path());
        assert!(r2.taps.iter().all(|t| t.new_files.is_empty()));
    }

    #[test]
    fn credential_file_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(
            &dir,
            &[
                ("ok.md", "hello"),
                ("bad.md", "key AKIAIOSFODNN7EXAMPLE end"),
            ],
        );
        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 1);
        assert_eq!(ing.skipped.len(), 1);
        assert_eq!(ing.skipped[0].path, "bad.md");
        assert_eq!(ing.skipped[0].pattern, "aws-access-key");
        assert_eq!(ing.skipped[0].preview, "AKIAIO***");
        let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
        assert!(!journal.contains("bad.md"));
    }

    #[test]
    fn ingest_advances_watermark() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "x")]);
        ingest::run(dir.path());
        let wm = std::fs::read_to_string(watermark_path(dir.path(), "harvest-dir")).unwrap();
        assert_eq!(wm.trim(), "1");
        let ing2 = ingest::run(dir.path());
        assert_eq!(ing2.added, 0);
    }
}

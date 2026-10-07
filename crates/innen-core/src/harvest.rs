//! DirectoryTap + harvest check + ingest.
//!
//! Pinned inbox: `<root>/00-inbox/harvest` (flat `*.md`, sorted).
//! Mapping: `id = "source:<sha8(relative_path)>"` (sha8 = first 8 hex of
//! `sha256_hex(relative_path)`); provenance `{path, bytes, sha256,
//! observed_utc}`; `idempotency_key` = full content sha256 hex; watermark =
//! files consumed count via [`crate::tap`] (`harvest-dir`).
//!
//! Known limitation (accepted tradeoff): the watermark is count-based, not
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
//!
//! ## Output contract and edge behavior
//!
//! `HarvestReport` / `IngestReport` / `TapReport` shapes are stable output
//! read by other tools — do not add, rename, or remove fields. New cases
//! must be reported through the existing fields.
//!
//! - IO failures surface as empty: unreadable inbox dir yields an empty
//!   report; [`check`] treats a per-file read failure as empty content;
//!   [`ingest::run`] skips an unreadable file while still advancing the
//!   watermark past it. They are not surfaced as errors because that would
//!   need new fields on the stable structs.
//! - Three UTF-8 policies: `Tap::collect` strict-aborts on non-UTF8
//!   (`TapError::Parse`); [`check`] reads with `unwrap_or_default` so a
//!   non-UTF8 file looks empty (hence clean); [`ingest::run`] scans
//!   `String::from_utf8_lossy`. They stay distinct because unifying them
//!   would change report shapes or the bytes/hashes recorded in reports.
//! - No file-size cap: files are read whole (`fs::read` /
//!   `read_to_string`); 64MB+ files will be slow. A cap would change the
//!   recorded bytes/hashes, so truncation/streaming needs a report-shape
//!   decision first.
//! - check-vs-ingest can disagree on unreadable / non-UTF8 files: check
//!   sees empty which scans clean, while ingest lossy-scans the real
//!   bytes and may skip for credentials. Accepted because the normal
//!   (clean UTF-8) path agrees (see
//!   `check_ingest_agree_on_clean_files`); reconciling the edge cases
//!   needs a report-shape decision first.

use std::path::{Path, PathBuf};

use crate::credentials::credential_preview;
use crate::tap::{Event, Tap, TapError};

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

/// Dry-run report for one tap.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TapReport {
    pub id: String,
    pub new_files: Vec<String>,
    pub skipped: Vec<String>,
    /// `missing` | `current` | `legacy_count_rebuilt` | `unreadable_rebuilt`.
    /// A rebuilt cursor means the inbox was re-listed in full, so the backlog
    /// below is authoritative rather than a continuation of an old count.
    pub cursor: String,
}

/// Dry-run report over all taps (currently exactly one, `harvest-dir`).
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

/// Inbox files this tap has not consumed, in listing order, plus the cursor
/// state that decided it.
///
/// [`check`] and [`ingest::run`] both select through here, so a dry run can
/// never disagree with the run it previews. A cursor that cannot name its
/// inputs (legacy count, unreadable file) contributes an empty consumed set:
/// re-listing everything is safe because the event id is content-derived and
/// the op is an upsert, whereas guessing a count would skip real files.
fn unconsumed(root: &Path) -> (Vec<String>, crate::tap::CursorState) {
    let state = crate::tap::load_cursor(root, TAP_ID);
    let cursor = state
        .known()
        .map(|consumed| crate::tap::Cursor {
            consumed: consumed.clone(),
        })
        .unwrap_or_default();
    let files = list_inbox_files(root);
    let pending = cursor
        .unconsumed(&files)
        .into_iter()
        .cloned()
        .collect::<Vec<String>>();
    (pending, state)
}

/// The consumed set to persist after this run: everything already recorded,
/// plus what this run handled. Not pruned — a name that leaves and comes back
/// must stay consumed rather than be appended twice.
fn advance(state: &crate::tap::CursorState, handled: &[String]) -> crate::tap::Cursor {
    let mut cursor = state
        .known()
        .map(|consumed| crate::tap::Cursor {
            consumed: consumed.clone(),
        })
        .unwrap_or_default();
    cursor.consumed.extend(handled.iter().cloned());
    cursor
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

/// DRY-RUN: no appends, no cursor advance.
pub fn check(root: &Path) -> HarvestReport {
    let (pending, state) = unconsumed(root);
    let mut new_files = Vec::new();
    let mut skipped = Vec::new();
    let inbox = root.join("00-inbox/harvest");
    for rel in pending {
        let text = std::fs::read_to_string(inbox.join(&rel)).unwrap_or_default();
        if crate::credentials::scan_credentials(&text).is_empty() {
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
            cursor: state.label().to_string(),
        }],
    }
}

pub mod ingest {
    use super::*;

    /// Append one `node.upsert` per clean file, skip credential hits
    /// (reported, never appended), then record every consumed name —
    /// added and skipped alike, so a credential hit is not retried forever
    /// and a clean file is never skipped by a shifted cursor.
    pub fn run(root: &Path) -> IngestReport {
        let (pending, state) = unconsumed(root);
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
        let mut processed: Vec<String> = Vec::with_capacity(total);
        for rel in &pending {
            let bytes = match std::fs::read(inbox.join(rel)) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let hits = crate::credentials::scan_credentials(&text);
            if let Some(pattern) = hits.first() {
                skipped.push(Skipped {
                    path: rel.clone(),
                    pattern: pattern.to_string(),
                    preview: credential_preview(&text, pattern),
                });
                processed.push(rel.clone());
                continue;
            }
            let event = event_for_file(rel, &bytes, &text);
            match journal.append(&event.op, &event.payload) {
                Ok(_) => {
                    added += 1;
                    processed.push(rel.clone());
                }
                Err(_) => break,
            }
        }
        if !processed.is_empty() {
            let cursor = advance(&state, &processed);
            let _ = crate::tap::store_cursor(root, TAP_ID, &cursor);
        }
        IngestReport { added, skipped }
    }
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
    fn ingest_skips_openai_and_upstash_with_labels() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(
            &dir,
            &[
                ("ok.md", "hello"),
                (
                    "ai.md",
                    "token sk-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop here",
                ),
                ("up.md", "UPSTASH_REDIS_REST_TOKEN=hunter2valuepayload end"),
            ],
        );
        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 1);
        assert_eq!(ing.skipped.len(), 2);
        assert_eq!(ing.skipped[0].pattern, "openai-key");
        assert_eq!(ing.skipped[0].preview, "sk-ABC***");
        assert_eq!(ing.skipped[1].pattern, "upstash-token");
        assert!(ing.skipped[1].preview.starts_with("UPSTAS"));
        let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
        assert!(!journal.contains("ai.md"));
        assert!(!journal.contains("up.md"));
    }

    #[test]
    fn ingest_records_the_consumed_file_names() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "x")]);
        ingest::run(dir.path());
        // The cursor names its inputs rather than counting them, which is what
        // keeps a shrinking inbox from swallowing new files.
        let state = crate::tap::load_cursor(dir.path(), "harvest-dir");
        assert_eq!(
            state,
            crate::tap::CursorState::Current(crate::tap::Cursor {
                consumed: ["a.md".to_string()].into_iter().collect()
            })
        );
        let ing2 = ingest::run(dir.path());
        assert_eq!(ing2.added, 0);
    }

    #[test]
    fn skipped_files_are_consumed_and_do_not_stall_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(
            &dir,
            &[
                ("bad.md", "key AKIAIOSFODNN7EXAMPLE end"),
                ("ok.md", "hello"),
            ],
        );
        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 1);
        assert_eq!(ing.skipped.len(), 1);
        let consumed = crate::tap::load_cursor(dir.path(), "harvest-dir")
            .known()
            .expect("cursor names its inputs")
            .clone();
        assert!(consumed.contains("bad.md"), "skipped files are consumed too");
        assert!(consumed.contains("ok.md"));
    }

    #[test]
    fn check_empty_inbox_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("00-inbox/harvest")).unwrap();
        let r = check(dir.path());
        assert!(r.taps.iter().all(|t| t.new_files.is_empty()));
        assert!(r.taps.iter().all(|t| t.skipped.is_empty()));
    }

    #[test]
    fn check_ingest_agree_on_clean_files() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "hello"), ("b.md", "world")]);
        let r = check(dir.path());
        assert_eq!(
            r.taps[0].new_files,
            vec!["a.md".to_string(), "b.md".to_string()]
        );
        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 2);
        assert!(ing.skipped.is_empty());
    }

    #[test]
    fn deleting_ingested_files_never_hides_new_ones() {
        // The failure this replaces: a count cursor plus post-ingest deletion.
        // Each deletion shifted the window by one, and once the count passed
        // the listing size `harvest --check` reported an empty backlog while
        // unprocessed files sat in the inbox — silently, for 17 days.
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "one"), ("b.md", "two")]);
        assert_eq!(ingest::run(dir.path()).added, 2);
        // The pending files' own instructions say to delete them after ingest.
        std::fs::remove_file(dir.path().join("00-inbox/harvest/a.md")).unwrap();
        std::fs::remove_file(dir.path().join("00-inbox/harvest/b.md")).unwrap();

        write_inbox(&dir, &[("c.md", "three"), ("d.md", "four")]);
        let r = check(dir.path());
        assert_eq!(
            r.taps[0].new_files,
            vec!["c.md".to_string(), "d.md".to_string()],
            "a shrunk inbox must not shift the window past new files"
        );
        assert_eq!(r.taps[0].cursor, "current");
        assert_eq!(ingest::run(dir.path()).added, 2);
        assert!(check(dir.path()).taps[0].new_files.is_empty());
    }

    #[test]
    fn many_ingest_delete_cycles_keep_finding_new_files() {
        let dir = tempfile::tempdir().unwrap();
        for round in 0..12 {
            let name = format!("pending-{round:04}.md");
            write_inbox(&dir, &[(name.as_str(), &format!("body {round}"))]);
            assert_eq!(
                ingest::run(dir.path()).added,
                1,
                "round {round} must ingest its new file"
            );
            std::fs::remove_file(dir.path().join("00-inbox/harvest").join(&name)).unwrap();
        }
        assert!(check(dir.path()).taps[0].new_files.is_empty());
    }

    #[test]
    fn a_legacy_count_cursor_rebuilds_and_says_so() {
        // The on-disk state this machine was stuck in: a count of 875 with 96
        // files present. It must re-list the inbox and report the rebuild
        // rather than presenting an empty backlog.
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "one"), ("b.md", "two")]);
        let path = watermark_path(dir.path(), "harvest-dir");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"875").unwrap();

        let r = check(dir.path());
        assert_eq!(r.taps[0].cursor, "legacy_count_rebuilt");
        assert_eq!(
            r.taps[0].new_files,
            vec!["a.md".to_string(), "b.md".to_string()]
        );

        let ing = ingest::run(dir.path());
        assert_eq!(ing.added, 2);
        assert_eq!(check(dir.path()).taps[0].cursor, "current");
        assert!(check(dir.path()).taps[0].new_files.is_empty());
    }

    #[test]
    fn consumed_names_survive_a_file_returning_to_the_inbox() {
        let dir = tempfile::tempdir().unwrap();
        write_inbox(&dir, &[("a.md", "same bytes")]);
        assert_eq!(ingest::run(dir.path()).added, 1);
        // Same name, same content, back in the inbox: the cursor still knows
        // it, so a retry loop cannot append it twice.
        write_inbox(&dir, &[("a.md", "same bytes")]);
        assert_eq!(ingest::run(dir.path()).added, 0);
        let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
        assert_eq!(journal.lines().count(), 1);
    }
}

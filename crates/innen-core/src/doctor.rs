//! Report-only doctor checks (Task 8b, spec §3).
//!
//! [`run`] inspects `<root>/.innen/` and returns a [`Report`] with four
//! checks in fixed order plus an exit code:
//!
//! | check | pass condition |
//! |---|---|
//! | `journal-valid` | journal file exists-or-absent and every non-empty line parses as a [`JournalEntry`] |
//! | `quarantine-list` | `quarantine/` absent, or holds no non-empty regular file |
//! | `index-rebuildable` | derived files (`index.redb` + `fts/`) both open with event counts matching the journal, or no derived index exists on a clean journal |
//! | `ref-integrity` | every non-retracted edge `from` is a known node id and every `to` is a known node id or a Uri |
//!
//! Exit mapping (implement exactly): `0` = all checks pass; `1` =
//! quarantine present OR index stale-but-rebuildable; `2` = journal invalid
//! OR ref-integrity broken (severity `2` dominates `1`).
//!
//! Edge mapping: quarantine-unreadable → `1` (via `quarantine-list` not-ok);
//! root-not-a-directory → `2` (via `journal-valid` not-ok).
//!
//! Report-only guarantee: NEVER mutates. This module only uses
//! [`std::fs::read`]/[`std::fs::read_to_string`]/[`std::fs::read_dir`] plus
//! [`tantivy::Index::open_in_dir`] and an in-memory redb open (file bytes via
//! [`std::fs::read`], never `redb::Database::open` on the original: that call
//! requires write access and bumps `index.redb` mtime even for a read txn,
//! which would break the no-mutation pins).
//! It never calls [`crate::journal::Journal::open`] (which would quarantine
//! bad lines and create dirs/lock files) nor [`crate::index::build`] /
//! [`crate::index::rebuild`]. Consequences, pinned by tests:
//!
//! * A garbage journal line reports exit `2` (journal invalid). A later
//!   `open()` by another command would quarantine that line, after which
//!   doctor reports exit `1` — doctor itself never performs the move.
//! * A missing derived index on a clean journal is `ok`: queries replay the
//!   journal directly, so there is nothing stale. A present-but-unopenable
//!   (or partial) derived index is stale-but-rebuildable → exit `1` when the
//!   journal is clean. A present-and-openable index whose event count has
//!   fallen behind the journal (post-build appends) is also
//!   stale-but-rebuildable → exit `1`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::journal::JournalEntry;

/// redb `events` table (mirrors [`crate::index`] layout: `id → raw entry JSON`).
const EVENTS: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("events");

/// One named check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Ordered check results plus the process exit code.
///
/// Serializes as `{checks:[{name, ok, detail}], exit_code}` (the human table
/// lives in the Task 8d CLI, not here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
    pub exit_code: u8,
}

fn innen_dir(root: &Path) -> PathBuf {
    root.join(".innen")
}

fn journal_path(root: &Path) -> PathBuf {
    innen_dir(root).join("journal.jsonl")
}

fn quarantine_dir(root: &Path) -> PathBuf {
    innen_dir(root).join("quarantine")
}

fn redb_path(root: &Path) -> PathBuf {
    innen_dir(root).join("index.redb")
}

fn fts_path(root: &Path) -> PathBuf {
    innen_dir(root).join("fts")
}

fn is_not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

/// `journal-valid`: the journal opens (read-only) and all lines parse.
///
/// A missing journal file is an empty journal (`ok`). Any non-empty line
/// that fails [`JournalEntry`] parsing makes the check fail.
fn check_journal(root: &Path) -> Check {
    let name = "journal-valid".to_string();
    if root.exists() && !root.is_dir() {
        return Check {
            name,
            ok: false,
            detail: format!("root is not a directory: {}", root.display()),
        };
    }
    let content = match fs::read_to_string(journal_path(root)) {
        Ok(content) => content,
        Err(e) if is_not_found(&e) => {
            return Check {
                name,
                ok: true,
                detail: "no journal file (empty)".to_string(),
            };
        }
        Err(e) => {
            return Check {
                name,
                ok: false,
                detail: format!("journal unreadable: {e}"),
            };
        }
    };
    let mut entries = 0usize;
    let mut bad = 0usize;
    let mut first_bad_line = 0usize;
    for (lineno, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        entries += 1;
        if serde_json::from_str::<JournalEntry>(line).is_err() {
            bad += 1;
            if first_bad_line == 0 {
                first_bad_line = lineno + 1;
            }
        }
    }
    if bad == 0 {
        Check {
            name,
            ok: true,
            detail: format!("{entries} entries, all parse"),
        }
    } else {
        Check {
            name,
            ok: false,
            detail: format!(
                "{bad} of {entries} lines unparseable (first at line {first_bad_line})"
            ),
        }
    }
}

/// `quarantine-list`: the quarantine dir is empty or absent (`ok`).
///
/// Any non-empty regular file inside counts as quarantine present.
fn check_quarantine(root: &Path) -> Check {
    let name = "quarantine-list".to_string();
    let dir = match fs::read_dir(quarantine_dir(root)) {
        Ok(dir) => dir,
        Err(e) if is_not_found(&e) => {
            return Check {
                name,
                ok: true,
                detail: "no quarantine dir".to_string(),
            };
        }
        Err(e) => {
            return Check {
                name,
                ok: false,
                detail: format!("quarantine unreadable: {e}"),
            };
        }
    };
    let mut present: Vec<String> = Vec::new();
    for entry in dir {
        let Ok(entry) = entry else {
            present.push("<unreadable-entry>".to_string());
            continue;
        };
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if !is_file {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(1);
        if len > 0 {
            present.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    present.sort();
    if present.is_empty() {
        Check {
            name,
            ok: true,
            detail: "quarantine empty/absent".to_string(),
        }
    } else {
        Check {
            name,
            ok: false,
            detail: format!("quarantine present: {}", present.join(", ")),
        }
    }
}

/// Parsed-event count in the journal: non-empty lines that parse as
/// [`JournalEntry`]. Missing file is `Some(0)`; `None` only when an existing
/// file cannot be read. Read-only (`fs::read_to_string`).
fn journal_parsed_count(root: &Path) -> Option<usize> {
    let content = match fs::read_to_string(journal_path(root)) {
        Ok(content) => content,
        Err(e) if is_not_found(&e) => return Some(0),
        Err(_) => return None,
    };
    let mut count = 0usize;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if serde_json::from_str::<JournalEntry>(line).is_ok() {
            count += 1;
        }
    }
    Some(count)
}

/// Row count of the redb `events` table from an already-open database.
/// `None` when the transaction or table cannot be read. Read-only (read txn).
fn redb_event_count_in(db: &redb::Database) -> Option<u64> {
    use redb::ReadableTableMetadata as _;
    let txn = db.begin_read().ok()?;
    let table = txn.open_table(EVENTS).ok()?;
    table.len().ok()
}

/// Open `index.redb` without touching the original file.
///
/// `redb::Database::open` requires write access and bumps the file mtime
/// even for a read-only transaction (verified: identical bytes, newer mtime),
/// which would violate the report-only + no-mutation pins. Instead read the
/// bytes (`fs::read`, `O_RDONLY`, no mtime change) into an in-memory backend
/// and open that: all redb writes land on the copy. `None` when the file
/// cannot be read, is empty, or is not a valid redb database (same
/// not-ok mapping as a failed open).
fn open_redb_readonly(path: &Path) -> Option<redb::Database> {
    use redb::StorageBackend as _;
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let backend = redb::backends::InMemoryBackend::new();
    backend.set_len(bytes.len() as u64).ok()?;
    backend.write(0, &bytes).ok()?;
    redb::Database::builder().create_with_backend(backend).ok()
}

/// `index-rebuildable`: derived files open and fresh, or nothing stale to rebuild.
///
/// * Both `index.redb` and `fts/` open **and** the redb `events` row count
///   matches the journal parsed-event count → `ok`.
/// * Both open but counts diverge (post-build appends) → not `ok`;
///   stale-but-rebuildable → exit `1` when the journal is clean.
/// * Neither exists → `ok` iff the journal is clean (nothing derived yet,
///   nothing stale; queries replay the journal directly).
/// * Otherwise (partial set, or present-but-unopenable) → not `ok`;
///   `journal_clean` decides whether the detail says `rebuildable`.
fn check_index(root: &Path, journal_clean: bool) -> Check {
    let name = "index-rebuildable".to_string();
    let rebuildable = if journal_clean {
        "rebuildable"
    } else {
        "not rebuildable (journal invalid)"
    };
    let redb = redb_path(root);
    let fts = fts_path(root);
    let redb_exists = redb.is_file();
    let fts_exists = fts.is_dir();
    if !redb_exists && !fts_exists {
        if journal_clean {
            return Check {
                name,
                ok: true,
                detail: "no derived index; journal replays clean".to_string(),
            };
        }
        return Check {
            name,
            ok: false,
            detail: format!("no derived index; {rebuildable}"),
        };
    }
    if redb_exists && fts_exists {
        // Single open per check: probe + count share one `Database` opened
        // from an in-memory copy (original mtime untouched).
        let (redb_ok, redb_count) = match open_redb_readonly(&redb) {
            Some(db) => {
                let count = redb_event_count_in(&db);
                (true, count)
            }
            None => (false, None),
        };
        let fts_ok = tantivy::Index::open_in_dir(&fts).is_ok();
        if redb_ok && fts_ok {
            if let (Some(index_n), Some(journal_m)) = (redb_count, journal_parsed_count(root)) {
                if index_n != journal_m as u64 {
                    return Check {
                        name,
                        ok: false,
                        detail: format!(
                            "stale index (index has {index_n} events, journal has {journal_m}); {rebuildable}"
                        ),
                    };
                }
            }
            return Check {
                name,
                ok: true,
                detail: "index.redb + fts/ open".to_string(),
            };
        }
        let mut broken = Vec::new();
        if !redb_ok {
            broken.push("index.redb");
        }
        if !fts_ok {
            broken.push("fts/");
        }
        return Check {
            name,
            ok: false,
            detail: format!(
                "stale index (unreadable: {}); {rebuildable}",
                broken.join(", ")
            ),
        };
    }
    let mut missing = Vec::new();
    if !redb_exists {
        missing.push("index.redb");
    }
    if !fts_exists {
        missing.push("fts/");
    }
    Check {
        name,
        ok: false,
        detail: format!(
            "partial derived index (missing: {}); {rebuildable}",
            missing.join(", ")
        ),
    }
}

/// Uri shape, mirroring [`crate::graph::validate`]: non-empty and starting
/// with `/` or containing `://`.
fn is_uri(s: &str) -> bool {
    !s.is_empty() && (s.starts_with('/') || s.contains("://"))
}

/// `ref-integrity`: every non-retracted edge endpoint resolves.
///
/// `from` must be a known node id; `to` must be a known node id or a Uri.
/// Only lines that parse are considered (unparseable lines already fail
/// `journal-valid`); retracted rows are history, not live references.
fn check_ref_integrity(root: &Path) -> Check {
    let name = "ref-integrity".to_string();
    let content = match fs::read_to_string(journal_path(root)) {
        Ok(content) => content,
        Err(e) if is_not_found(&e) => {
            return Check {
                name,
                ok: true,
                detail: "no edges (empty journal)".to_string(),
            };
        }
        Err(e) => {
            return Check {
                name,
                ok: false,
                detail: format!("journal unreadable: {e}"),
            };
        }
    };
    let mut events = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JournalEntry>(line) else {
            continue;
        };
        events.push(json!({
            "op": entry.op,
            "payload": entry.payload,
            "observed_utc": entry.observed_utc,
        }));
    }
    let materialized = crate::graph::materialize(&events, None, true);
    let mut dangling: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for edge in materialized.edges.iter().filter(|e| !e.retracted) {
        checked += 1;
        if !materialized.nodes.contains_key(&edge.from) {
            dangling.push(format!(
                "from '{}' -[{}]-> '{}' (unknown from)",
                edge.from, edge.edge, edge.to
            ));
            continue;
        }
        if !is_uri(&edge.to) && !materialized.nodes.contains_key(&edge.to) {
            dangling.push(format!(
                "from '{}' -[{}]-> '{}' (unknown to)",
                edge.from, edge.edge, edge.to
            ));
        }
    }
    if dangling.is_empty() {
        Check {
            name,
            ok: true,
            detail: format!("{checked} edges, all resolve"),
        }
    } else {
        let mut shown: String = dangling
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        if dangling.len() > 5 {
            shown += &format!("; and {} more", dangling.len() - 5);
        }
        Check {
            name,
            ok: false,
            detail: format!("{} dangling edge(s): {shown}", dangling.len()),
        }
    }
}

/// Run the four checks in order and fold the exit code.
///
/// `0` = all pass; `1` = quarantine present OR index stale-but-rebuildable;
/// `2` = journal invalid OR ref-integrity broken. Report-only: performs no
/// writes (no rebuild, no quarantine moves).
pub fn run(root: &Path) -> Report {
    let journal = check_journal(root);
    let quarantine = check_quarantine(root);
    let index = check_index(root, journal.ok);
    let refs = check_ref_integrity(root);
    let exit_code = if !journal.ok || !refs.ok {
        2
    } else if !quarantine.ok || !index.ok {
        1
    } else {
        0
    };
    Report {
        checks: vec![journal, quarantine, index, refs],
        exit_code,
    }
}

/// Lint: journal-valid + quarantine-empty + ref-integrity subset of [`run`].
///
/// Placement: lives here (not `config.rs`) to reuse the private read-only
/// checks without widening visibility or adding a module. The quarantine check
/// reuses [`check_quarantine`] logic but is renamed to `quarantine-empty` per
/// the Task 8c scope wording. Order: `journal-valid`, `quarantine-empty`,
/// `ref-integrity`. Exits mirror [`run`]: `2` when journal-invalid or
/// ref-integrity broken (dominates), `1` when quarantine non-empty, else `0`.
/// Report-only: no writes (same guarantees as [`run`]).
pub fn lint(root: &Path) -> Report {
    let journal = check_journal(root);
    let mut quarantine = check_quarantine(root);
    quarantine.name = "quarantine-empty".to_string();
    let refs = check_ref_integrity(root);
    let exit_code = if !journal.ok || !refs.ok {
        2
    } else if !quarantine.ok {
        1
    } else {
        0
    };
    Report {
        checks: vec![journal, quarantine, refs],
        exit_code,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;

    use serde_json::json;

    use crate::journal::Journal;

    use super::run;

    /// Shared fixture: two nodes plus one valid edge, all via
    /// [`Journal::append`] (no hand-written journal bytes).
    fn fixture_two_nodes_one_edge(root: &std::path::Path) {
        let journal = Journal::open(root).expect("open");
        journal
            .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
            .expect("append n:1");
        journal
            .append("node.upsert", &json!({"id": "n:2", "label": "two"}))
            .expect("append n:2");
        journal
            .append(
                "edge.assert",
                &json!({"from": "n:1", "to": "n:2", "type": "FOLLOWS_UP"}),
            )
            .expect("append edge");
    }

    fn journal_bytes(root: &std::path::Path) -> Vec<u8> {
        fs::read(root.join(".innen/journal.jsonl")).expect("read journal bytes")
    }

    /// Sorted recursive listing of `.innen` (relative paths) for no-mutation pins.
    fn snapshot_innen(root: &std::path::Path) -> Vec<String> {
        let base = root.join(".innen");
        let mut out = Vec::new();
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir snapshot") {
                let entry = entry.expect("dir entry snapshot");
                let path = entry.path();
                let rel = path
                    .strip_prefix(&base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.push(rel);
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn doctor_healthy_is_0() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        let before = journal_bytes(dir.path());

        let report = run(dir.path());

        assert_eq!(report.exit_code, 0, "healthy repo must exit 0: {report:?}");
        let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "journal-valid",
                "quarantine-list",
                "index-rebuildable",
                "ref-integrity"
            ],
            "checks in spec order: {report:?}"
        );
        for check in &report.checks {
            assert!(check.ok, "{} must be ok: {}", check.name, check.detail);
        }

        // JSON shape pin: {checks:[{name, ok, detail}], exit_code}.
        let value = serde_json::to_value(&report).expect("report serializes");
        assert_eq!(value.get("exit_code").and_then(|v| v.as_u64()), Some(0));
        let checks = value
            .get("checks")
            .and_then(|v| v.as_array())
            .expect("checks is an array");
        assert_eq!(checks.len(), 4);
        assert_eq!(
            checks[0].get("name").and_then(|v| v.as_str()),
            Some("journal-valid")
        );

        // Report-only: journal bytes untouched, no derived index built.
        assert_eq!(journal_bytes(dir.path()), before);
        assert!(!dir.path().join(".innen/index.redb").exists());
        assert!(!dir.path().join(".innen/fts").exists());
    }

    #[test]
    fn doctor_quarantine_is_1() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        // Preseed one corrupt line *in the quarantine dir*. The journal
        // itself stays valid: journal corruption is exit 2, while quarantine
        // presence alone is exit 1.
        let qdir = dir.path().join(".innen/quarantine");
        fs::create_dir_all(&qdir).expect("mkdir quarantine");
        fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed quarantine");
        let before = journal_bytes(dir.path());

        let report = run(dir.path());

        assert_eq!(
            report.exit_code, 1,
            "quarantine present must exit 1: {report:?}"
        );
        assert_eq!(report.checks[1].name, "quarantine-list");
        assert!(
            !report.checks[1].ok,
            "quarantine-list must be not-ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok: {}",
            report.checks[0].detail
        );

        // Report-only: journal and preseeded quarantine both untouched.
        assert_eq!(journal_bytes(dir.path()), before);
        assert_eq!(
            fs::read_to_string(qdir.join("2026-01-01.jsonl")).expect("reread quarantine"),
            "not json at all\n"
        );
    }

    #[test]
    fn doctor_stale_index_is_1() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        crate::index::build(dir.path()).expect("build");
        // Post-build append: the journal moves on, the derived index falls
        // behind (3 indexed events vs 4 journal events).
        let journal = Journal::open(dir.path()).expect("reopen");
        journal
            .append("node.upsert", &json!({"id": "n:3", "label": "three"}))
            .expect("append n:3");
        drop(journal);
        let before = journal_bytes(dir.path());
        // No-mutation pins with a present index: mtimes + full listing.
        let redb_file = dir.path().join(".innen/index.redb");
        let fts_dir = dir.path().join(".innen/fts");
        let redb_mtime = fs::metadata(&redb_file)
            .expect("redb meta")
            .modified()
            .expect("redb mtime");
        let fts_mtime = fs::metadata(&fts_dir)
            .expect("fts meta")
            .modified()
            .expect("fts mtime");
        let before_listing = snapshot_innen(dir.path());

        let report = run(dir.path());

        assert_eq!(report.exit_code, 1, "stale index must exit 1: {report:?}");
        assert_eq!(report.checks[2].name, "index-rebuildable");
        assert!(
            !report.checks[2].ok,
            "index-rebuildable must be not-ok: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2]
                .detail
                .contains("index has 3 events, journal has 4"),
            "detail names both counts: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok: {}",
            report.checks[0].detail
        );
        assert!(
            report.checks[1].ok,
            "quarantine-list stays ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[3].ok,
            "ref-integrity stays ok: {}",
            report.checks[3].detail
        );

        // Report-only: the stale report leaves the journal untouched.
        assert_eq!(journal_bytes(dir.path()), before);
        // Report-only with a present index: derived bytes untouched.
        assert_eq!(
            fs::metadata(&redb_file)
                .expect("reread redb meta")
                .modified()
                .expect("reread redb mtime"),
            redb_mtime,
            "index.redb mtime must not change"
        );
        assert_eq!(
            fs::metadata(&fts_dir)
                .expect("reread fts meta")
                .modified()
                .expect("reread fts mtime"),
            fts_mtime,
            "fts/ mtime must not change"
        );
        assert_eq!(
            snapshot_innen(dir.path()),
            before_listing,
            "no files added/removed under .innen"
        );

        // Rebuild converges: a fresh index matches the journal again.
        crate::index::rebuild(dir.path()).expect("rebuild");
        let report = run(dir.path());
        assert_eq!(report.exit_code, 0, "rebuilt index must exit 0: {report:?}");
        for check in &report.checks {
            assert!(check.ok, "{} must be ok: {}", check.name, check.detail);
        }
    }

    #[test]
    fn doctor_dangling_edge_is_2() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        journal
            .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
            .expect("append n:1");
        // Dangling edge: `to` resolves to neither a known node id nor a Uri.
        // Note: unparseable journal lines cannot produce exit 2 through
        // `Journal::open` (it quarantines them into exit 1), so exit 2 is
        // pinned via ref-integrity instead.
        journal
            .append(
                "edge.assert",
                &json!({"from": "n:1", "to": "n:ghost", "type": "FOLLOWS_UP"}),
            )
            .expect("append dangling edge");
        drop(journal);

        let report = run(dir.path());

        assert_eq!(report.exit_code, 2, "dangling edge must exit 2: {report:?}");
        assert_eq!(report.checks[3].name, "ref-integrity");
        assert!(
            !report.checks[3].ok,
            "ref-integrity must be not-ok: {}",
            report.checks[3].detail
        );
        assert!(
            report.checks[3].detail.contains("n:ghost"),
            "detail names the dangling endpoint: {}",
            report.checks[3].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok (dangling edge parses): {}",
            report.checks[0].detail
        );
        assert!(
            report.checks[1].ok,
            "quarantine-list stays ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[2].ok,
            "index-rebuildable stays ok (no derived index, clean journal): {}",
            report.checks[2].detail
        );
    }

    #[test]
    fn exit_2_dominates_1() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        journal
            .append("node.upsert", &json!({"id": "n:1", "label": "one"}))
            .expect("append n:1");
        // Dangling edge pins exit 2 via ref-integrity (see
        // `doctor_dangling_edge_is_2`).
        journal
            .append(
                "edge.assert",
                &json!({"from": "n:1", "to": "n:ghost", "type": "FOLLOWS_UP"}),
            )
            .expect("append dangling edge");
        drop(journal);
        // Quarantine presence alone pins exit 1; severity 2 must dominate.
        let qdir = dir.path().join(".innen/quarantine");
        fs::create_dir_all(&qdir).expect("mkdir quarantine");
        fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed quarantine");

        let report = run(dir.path());

        assert_eq!(
            report.exit_code, 2,
            "exit 2 must dominate exit 1: {report:?}"
        );
        assert!(
            !report.checks[1].ok,
            "quarantine-list must be not-ok: {}",
            report.checks[1].detail
        );
        assert!(
            !report.checks[3].ok,
            "ref-integrity must be not-ok: {}",
            report.checks[3].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok (dangling edge parses): {}",
            report.checks[0].detail
        );
        assert!(
            report.checks[2].ok,
            "index-rebuildable stays ok (no derived index, clean journal): {}",
            report.checks[2].detail
        );
    }

    #[test]
    fn partial_index_reports() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        crate::index::build(dir.path()).expect("build");
        // Leave a partial derived index: index.redb present, fts/ missing.
        fs::remove_dir_all(dir.path().join(".innen/fts")).expect("remove fts");
        assert!(dir.path().join(".innen/index.redb").is_file());
        assert!(!dir.path().join(".innen/fts").exists());

        let report = run(dir.path());

        assert_eq!(report.exit_code, 1, "partial index must exit 1: {report:?}");
        assert_eq!(report.checks[2].name, "index-rebuildable");
        assert!(
            !report.checks[2].ok,
            "index-rebuildable must be not-ok: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2].detail.contains("partial"),
            "detail says partial: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2].detail.contains("fts/"),
            "detail names the missing side: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2].detail.contains("rebuildable"),
            "clean journal is rebuildable: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok: {}",
            report.checks[0].detail
        );
        assert!(
            report.checks[1].ok,
            "quarantine-list stays ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[3].ok,
            "ref-integrity stays ok: {}",
            report.checks[3].detail
        );
    }

    #[test]
    fn present_unopenable_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        crate::index::build(dir.path()).expect("build");
        // Present-but-unopenable redb alongside a valid fts/ dir.
        fs::write(dir.path().join(".innen/index.redb"), b"garbage-not-redb").expect("corrupt redb");

        let report = run(dir.path());

        assert_eq!(
            report.exit_code, 1,
            "unopenable index must exit 1: {report:?}"
        );
        assert_eq!(report.checks[2].name, "index-rebuildable");
        assert!(
            !report.checks[2].ok,
            "index-rebuildable must be not-ok: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2].detail.contains("index.redb"),
            "detail names the unreadable side: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[2].detail.contains("rebuildable"),
            "clean journal is rebuildable: {}",
            report.checks[2].detail
        );
        assert!(
            !report.checks[2].detail.contains("not rebuildable"),
            "clean journal must not say not-rebuildable: {}",
            report.checks[2].detail
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok: {}",
            report.checks[0].detail
        );
        assert!(
            report.checks[1].ok,
            "quarantine-list stays ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[3].ok,
            "ref-integrity stays ok: {}",
            report.checks[3].detail
        );
    }

    #[test]
    fn doctor_never_mutates() {
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        // Sabotage the journal tail directly: run() must report it, not heal it.
        let journal_path = dir.path().join(".innen/journal.jsonl");
        {
            let mut f = fs::OpenOptions::new()
                .append(true)
                .open(&journal_path)
                .expect("reopen journal");
            f.write_all(b"garbage-not-json\n").expect("sabotage");
        }
        let before = fs::read(&journal_path).expect("journal bytes");
        assert!(before.ends_with(b"garbage-not-json\n"));

        let report = run(dir.path());

        assert_eq!(
            report.exit_code, 2,
            "corrupt journal must exit 2: {report:?}"
        );
        assert!(
            !report.checks[0].ok,
            "journal-valid must be not-ok: {}",
            report.checks[0].detail
        );
        // Nothing healed, nothing built: corrupt bytes still in place, no
        // quarantine move, no derived index.
        assert_eq!(fs::read(&journal_path).expect("reread journal"), before);
        assert!(
            !dir.path().join(".innen/quarantine").exists(),
            "run must not quarantine"
        );
        assert!(!dir.path().join(".innen/index.redb").exists());
        assert!(!dir.path().join(".innen/fts").exists());
    }

    #[test]
    fn lint_reports_quarantine() {
        // Preseeded quarantine file → lint exit 1.
        let dir = tempfile::tempdir().expect("tempdir");
        fixture_two_nodes_one_edge(dir.path());
        let qdir = dir.path().join(".innen/quarantine");
        fs::create_dir_all(&qdir).expect("mkdir quarantine");
        fs::write(qdir.join("2026-01-01.jsonl"), "not json at all\n").expect("preseed quarantine");

        let report = super::lint(dir.path());

        assert_eq!(
            report.exit_code, 1,
            "quarantine present must exit 1: {report:?}"
        );
        let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["journal-valid", "quarantine-empty", "ref-integrity"],
            "lint scope in order: {report:?}"
        );
        assert!(
            report.checks[0].ok,
            "journal-valid stays ok: {}",
            report.checks[0].detail
        );
        assert!(
            !report.checks[1].ok,
            "quarantine-empty must be not-ok: {}",
            report.checks[1].detail
        );
        assert!(
            report.checks[2].ok,
            "ref-integrity stays ok: {}",
            report.checks[2].detail
        );
    }
}

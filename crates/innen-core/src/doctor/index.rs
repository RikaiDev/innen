//! `index-rebuildable`: are the derived files fresh, or nothing stale?

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::journal::JournalEntry;

use super::paths::{fts_path, is_not_found, journal_path, redb_path};
use super::Check;

/// redb `events` table (mirrors [`crate::index`] layout: `id → raw entry JSON`).
const EVENTS: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("events");

fn journal_expected_events(root: &Path) -> Option<BTreeMap<String, String>> {
    let content = match fs::read_to_string(journal_path(root)) {
        Ok(content) => content,
        Err(e) if is_not_found(&e) => return Some(BTreeMap::new()),
        Err(_) => return None,
    };
    let mut expected = BTreeMap::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let entry = serde_json::from_str::<JournalEntry>(line).ok()?;
        let raw = serde_json::to_string(&entry).ok()?;
        expected.insert(entry.id, raw);
    }
    Some(expected)
}

fn redb_matches_expected(
    db: &redb::Database,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    use redb::{ReadableDatabase as _, ReadableTableMetadata as _};
    let txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = txn.open_table(EVENTS).map_err(|e| e.to_string())?;
    let actual = table.len().map_err(|e| e.to_string())?;
    if actual != expected.len() as u64 {
        return Err(format!(
            "event key count differs (index has {actual} events, journal has {} unique IDs)",
            expected.len()
        ));
    }
    for (id, raw_expected) in expected {
        let raw_actual = table
            .get(id.as_str())
            .map_err(|e| e.to_string())?
            .map(|value| value.value().to_owned());
        if raw_actual.as_deref() != Some(raw_expected.as_str()) {
            return Err(format!("event {id} content differs"));
        }
    }
    Ok(())
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
pub(super) fn check_index(root: &Path, journal_clean: bool) -> Check {
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
        let db = open_redb_readonly(&redb);
        let redb_ok = db.is_some();
        let fts_ok = tantivy::Index::open_in_dir(&fts).is_ok();
        if redb_ok && fts_ok {
            if let Some(expected) = journal_expected_events(root) {
                if let Err(reason) = redb_matches_expected(db.as_ref().expect("redb_ok"), &expected)
                {
                    return Check {
                        name,
                        ok: false,
                        detail: format!("stale index ({reason}); {rebuildable}"),
                    };
                }
            } else {
                return Check {
                    name,
                    ok: false,
                    detail: format!("stale index (journal freshness unavailable); {rebuildable}"),
                };
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

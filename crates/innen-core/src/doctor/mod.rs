//! Report-only doctor checks.
//!
//! [`run`] inspects `<root>/.innen/` and returns a [`Report`] with four
//! checks in fixed order plus an exit code:
//!
//! | check | pass condition |
//! |---|---|
//! | `journal-valid` | journal file exists-or-absent and every non-empty line parses as a [`JournalEntry`](crate::journal::JournalEntry) |
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

mod index;
mod journal;
mod paths;
mod quarantine;
mod refs;
mod tap;

#[cfg(test)]
mod tests;

use std::path::Path;

use serde::{Deserialize, Serialize};

use index::check_index;
use journal::check_journal;
use quarantine::check_quarantine;
use refs::check_ref_integrity;
use tap::check_tap_cursor;

/// One named check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Ordered check results plus the process exit code.
///
/// Serializes as `{checks:[{name, ok, detail}], exit_code}` (the human table
/// lives in the CLI, not here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub checks: Vec<Check>,
    pub exit_code: u8,
}

/// `2` = journal invalid OR ref-integrity broken. Report-only: performs no
/// writes (no rebuild, no quarantine moves).
pub fn run(root: &Path) -> Report {
    let journal = check_journal(root);
    let quarantine = check_quarantine(root);
    let index = check_index(root, journal.ok);
    let refs = check_ref_integrity(root);
    let cursor = check_tap_cursor(root);
    let exit_code = if !journal.ok || !refs.ok {
        2
    } else if !quarantine.ok || !index.ok || !cursor.ok {
        1
    } else {
        0
    };
    Report {
        checks: vec![journal, quarantine, index, refs, cursor],
        exit_code,
    }
}

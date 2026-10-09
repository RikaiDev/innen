//! `journal-valid`.

use std::fs;
use std::path::Path;

use crate::journal::JournalEntry;

use super::paths::{is_not_found, journal_path};
use super::Check;

/// `journal-valid`: the journal opens (read-only) and all lines parse.
///
/// A missing journal file is an empty journal (`ok`). Any non-empty line
/// that fails [`JournalEntry`] parsing makes the check fail.
pub(super) fn check_journal(root: &Path) -> Check {
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

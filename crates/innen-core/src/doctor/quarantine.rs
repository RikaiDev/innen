//! `quarantine-list`.

use std::fs;
use std::path::Path;

use super::paths::{is_not_found, quarantine_dir};
use super::Check;

/// `quarantine-list`: the quarantine dir is empty or absent (`ok`).
///
/// Any non-empty regular file inside counts as quarantine present.
pub(super) fn check_quarantine(root: &Path) -> Check {
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

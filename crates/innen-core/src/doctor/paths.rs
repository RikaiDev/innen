//! On-disk layout of `<root>/.innen/` the checks read.

use std::path::{Path, PathBuf};

fn innen_dir(root: &Path) -> PathBuf {
    root.join(".innen")
}

pub(super) fn journal_path(root: &Path) -> PathBuf {
    innen_dir(root).join("journal.jsonl")
}

pub(super) fn quarantine_dir(root: &Path) -> PathBuf {
    innen_dir(root).join("quarantine")
}

pub(super) fn redb_path(root: &Path) -> PathBuf {
    innen_dir(root).join("index.redb")
}

pub(super) fn fts_path(root: &Path) -> PathBuf {
    innen_dir(root).join("fts")
}

pub(super) fn is_not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

//! Filesystem primitives: hashing, locking, atomic writes, path resolution.

use fs2::FileExt;
use sha2::Digest;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::LedgerError;

pub(super) fn hash_file(p: &Path) -> Result<(String, u64), std::io::Error> {
    let mut f = File::open(p)?;
    let mut h = sha2::Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    let mut n = 0;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64
    }
    let d = h.finalize();
    let s = d.iter().map(|b| format!("{b:02x}")).collect();
    Ok((s, n))
}
pub(super) fn lock(p: &Path) -> Result<File, LedgerError> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(p)
        .map_err(io)?;
    f.lock_exclusive().map_err(io)?;
    Ok(f)
}
pub(super) fn atomic(p: &Path, b: &[u8]) -> Result<(), LedgerError> {
    let t = p.with_file_name(format!(
        ".{}.tmp-{}",
        p.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&t)
        .map_err(io)?;
    f.write_all(b).map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::rename(&t, p).map_err(io)
}
pub(super) fn atomic_create(p: &Path, b: &[u8]) -> Result<(), LedgerError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .map_err(io)?;
    f.write_all(b).map_err(io)?;
    f.sync_all().map_err(io)
}
pub(super) fn io(e: std::io::Error) -> LedgerError {
    LedgerError::Io(e.to_string())
}
pub(super) fn absolute(p: &Path) -> Result<PathBuf, LedgerError> {
    if !p.is_absolute() {
        return Err(LedgerError::Invalid(format!(
            "path must be absolute: {}",
            p.display()
        )));
    }
    let mut tail = PathBuf::new();
    let mut cur = p;
    while !cur.exists() {
        let n = cur
            .parent()
            .ok_or_else(|| LedgerError::Invalid("cannot resolve path".into()))?;
        tail = cur
            .file_name()
            .map(|x| {
                let mut q = PathBuf::from(x);
                q.push(&tail);
                q
            })
            .unwrap_or(tail);
        cur = n
    }
    let mut out = fs::canonicalize(cur).map_err(io)?;
    if !tail.as_os_str().is_empty() {
        out.push(tail);
    }
    Ok(out)
}
pub(super) fn under(p: &Path, b: &Path) -> bool {
    p == b || p.strip_prefix(b).is_ok()
}
pub(super) fn default_downloads() -> Result<PathBuf, LedgerError> {
    absolute(
        &std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Downloads"),
    )
}

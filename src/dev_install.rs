//! Retire a stale dev build.
//!
//! `scripts/dev-install.sh` puts a working-tree build in `~/.local/bin`, which
//! shadows Homebrew on PATH, and records it in
//! `~/.local/share/innen/dev-install.json`. Once Homebrew holds a newer official
//! release, that shadow only hides the release. When the running binary is that
//! recorded dev build, it came from a clean tree and a newer Homebrew version is
//! installed, this removes the dev build and hands the current invocation to the
//! official binary. Anything unexpected leaves the files alone.

use std::path::{Path, PathBuf};

const OFFICIAL_PREFIXES: [&str; 2] = ["/opt/homebrew", "/usr/local"];

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

fn newest_official(prefixes: &[&str]) -> Option<((u64, u64, u64), PathBuf)> {
    prefixes
        .iter()
        .filter_map(|prefix| {
            let bin = Path::new(prefix).join("bin/innen");
            let cellar = Path::new(prefix).join("Cellar/innen");
            let newest = std::fs::read_dir(cellar)
                .ok()?
                .filter_map(|entry| parse_version(&entry.ok()?.file_name().to_string_lossy()))
                .max()?;
            bin.exists().then_some((newest, bin))
        })
        .max_by_key(|(version, _)| *version)
}

/// Returns the path of the official binary to run instead, after removing the
/// stale dev build, or `None` to continue in this process.
fn retire(exe: &Path, marker: &Path, prefixes: &[&str]) -> Option<PathBuf> {
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(marker).ok()?).ok()?;
    let recorded = PathBuf::from(record.get("path")?.as_str()?);
    if std::fs::canonicalize(&recorded).ok()? != std::fs::canonicalize(exe).ok()? {
        return None;
    }
    if record.get("dirty")?.as_bool()? {
        return None;
    }
    let dev_version = parse_version(record.get("version")?.as_str()?)?;
    let (official_version, official_bin) = newest_official(prefixes)?;
    if official_version <= dev_version {
        return None;
    }
    std::fs::remove_file(&recorded).ok()?;
    let _ = std::fs::remove_file(marker);
    eprintln!(
        "innen: removed dev build {} ({}.{}.{}); Homebrew has {}.{}.{}",
        recorded.display(),
        dev_version.0,
        dev_version.1,
        dev_version.2,
        official_version.0,
        official_version.1,
        official_version.2
    );
    Some(official_bin)
}

/// Called first in `main`: if this process is a stale dev build, retire it and
/// exec the official binary with the same arguments.
pub fn handoff_if_stale() {
    let (Ok(exe), Some(home)) = (std::env::current_exe(), std::env::var_os("HOME")) else {
        return;
    };
    let marker = Path::new(&home).join(".local/share/innen/dev-install.json");
    if !marker.exists() {
        return;
    }
    if let Some(official) = retire(&exe, &marker, &OFFICIAL_PREFIXES) {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(&official)
                .args(std::env::args_os().skip(1))
                .exec();
            eprintln!("innen: could not start {}: {error}", official.display());
            std::process::exit(127);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(
        dev_version: &str,
        official: &str,
        dirty: bool,
    ) -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("local/bin/innen");
        std::fs::create_dir_all(dev.parent().unwrap()).unwrap();
        std::fs::write(&dev, b"dev").unwrap();
        let prefix = dir.path().join("brew");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(prefix.join("bin/innen"), b"official").unwrap();
        std::fs::create_dir_all(prefix.join("Cellar/innen").join(official)).unwrap();
        let marker = dir.path().join("dev-install.json");
        std::fs::write(
            &marker,
            serde_json::json!({"path": dev, "version": dev_version, "dirty": dirty}).to_string(),
        )
        .unwrap();
        let prefix = prefix.to_string_lossy().into_owned();
        (dir, dev, marker, prefix)
    }

    #[test]
    fn newer_official_release_retires_a_clean_dev_build() {
        let (_dir, dev, marker, prefix) = fixture("0.8.0", "0.9.0", false);
        let official = retire(&dev, &marker, &[prefix.as_str()]).expect("handoff");
        assert!(official.ends_with("bin/innen"));
        assert!(!dev.exists() && !marker.exists());
    }

    #[test]
    fn same_version_dirty_or_foreign_binary_is_left_alone() {
        let (_d1, dev, marker, prefix) = fixture("0.9.0", "0.9.0", false);
        assert!(retire(&dev, &marker, &[prefix.as_str()]).is_none());
        assert!(dev.exists());
        let (_d2, dev, marker, prefix) = fixture("0.8.0", "0.9.0", true);
        assert!(retire(&dev, &marker, &[prefix.as_str()]).is_none());
        assert!(dev.exists());
        let (d3, _dev, marker, prefix) = fixture("0.8.0", "0.9.0", false);
        let other = d3.path().join("other-innen");
        std::fs::write(&other, b"x").unwrap();
        assert!(retire(&other, &marker, &[prefix.as_str()]).is_none());
    }
}

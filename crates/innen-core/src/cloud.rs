//! Stubbed rclone cloud harness (Task 11b).
//!
//! Thin subprocess wrapper over a `rclone` binary (constructed by the CLI
//! via `PATH` lookup; tests pass the fixture path directly). No network in
//! tests: all subprocesses exec the local bash stub at
//! `crates/innen-core/tests/fixtures/fake-rclone.sh`.
//!
//! Fixture path note: unit tests live in the core crate, so `fixture()`
//! resolves `tests/fixtures/fake-rclone.sh` via `CARGO_MANIFEST_DIR` —
//! i.e. `crates/innen-core/tests/fixtures/` — not workspace-root
//! `tests/fixtures/`. The committed `git add` path is adjusted accordingly.
//!
//! Subprocess trust boundary: [`run`] spawns with piped stdout/stderr,
//! writes optional stdin, then polls [`std::process::Child::try_wait`] every
//! 5ms up to 15s, killing on expiry. The stub is instant, so this never
//! fires in tests. Follow-up for production use: large `cat` outputs can
//! fill the pipe buffer while we poll (child blocks on write, parent waits
//! for exit → spurious timeout); stream `cat` stdout to the dest file and
//! `rcat` stdin from the source file instead of buffering whole files.
//!
//! Evict mapping: eviction is `prune --apply` against the manifest. The only
//! deletion path in this module is [`Rclone::prune`] with `apply = true`
//! (the sole caller of rclone `delete`); no other method deletes.
//!
//! Manifest format for [`Rclone::candidates`]: `<root>/.innen/cloud-manifest.json`
//! as a JSON array of strings (relative posix paths, e.g.
//! `["a.bin","sub/b.bin"]`) or `{"files": [...]}`. Missing file → empty set,
//! so all local files are candidates (per spec). Unparseable → `Err`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Subprocess timeout (stub is instant; real rclone hangs fail closed here).
const TIMEOUT: Duration = Duration::from_secs(15);

/// Spawn `bin args`, feed optional stdin, poll with timeout, kill on expiry.
fn run(
    bin: &Path,
    args: &[&str],
    stdin_bytes: Option<&[u8]>,
) -> Result<std::process::Output, String> {
    use std::io::{Read as _, Write as _};
    use std::process::Stdio;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args);
    if stdin_bytes.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    if let Some(bytes) = stdin_bytes {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(bytes)
                .map_err(|e| format!("write stdin: {e}"))?;
        }
    }
    let start = Instant::now();
    let status = loop {
        match child
            .try_wait()
            .map_err(|e| format!("wait {}: {e}", bin.display()))?
        {
            Some(s) => break s,
            None => {
                if start.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "timeout after {}s: {} {}",
                        TIMEOUT.as_secs(),
                        bin.display(),
                        args.join(" ")
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout)
            .map_err(|e| format!("read stdout: {e}"))?;
    }
    if let Some(mut err) = child.stderr.take() {
        err.read_to_end(&mut stderr)
            .map_err(|e| format!("read stderr: {e}"))?;
    }
    let _ = child.wait();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Map a finished child to `Ok(stdout bytes)` / `Err(stderr text)`.
fn ok_or_stderr(
    output: std::process::Output,
    bin: &Path,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!(
                "{} {} failed: {}",
                bin.display(),
                args.join(" "),
                output.status
            ))
        } else {
            Err(stderr)
        }
    }
}

pub struct Rclone {
    pub bin: PathBuf,
}

impl Rclone {
    /// Runs `lsd <remote>:` → `Ok(stdout)` or `Err(stderr)`.
    pub fn status(&self, remote: &str) -> Result<String, String> {
        let target = format!("{remote}:");
        let args = ["lsd", target.as_str()];
        let out = run(&self.bin, &args, None)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Runs `rcat <remote>/<filename>` with the local file bytes on stdin.
    pub fn put(&self, local: &Path, remote: &str) -> Result<String, String> {
        let name = local
            .file_name()
            .and_then(|o| o.to_str())
            .ok_or_else(|| format!("put: local has no file name: {}", local.display()))?;
        let bytes = std::fs::read(local).map_err(|e| format!("read {}: {e}", local.display()))?;
        let target = format!("{remote}/{name}");
        let args = ["rcat", target.as_str()];
        let out = run(&self.bin, &args, Some(&bytes))?;
        let stdout = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    /// Runs `cat <remote-path>` → writes stdout bytes to `dest`, returns len.
    pub fn get(&self, remote_path: &str, dest: &Path) -> Result<u64, String> {
        let args = ["cat", remote_path];
        let out = run(&self.bin, &args, None)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create_dir {}: {e}", parent.display()))?;
            }
        }
        std::fs::write(dest, &bytes).map_err(|e| format!("write {}: {e}", dest.display()))?;
        Ok(bytes.len() as u64)
    }

    /// Binary responds to `version`, else `Err` (missing binary → spawn `Err`).
    pub fn doctor(&self) -> Result<String, String> {
        let args = ["version"];
        let out = run(&self.bin, &args, None)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// List remote orphans via `lsjson <remote>:`; `apply = true` deletes each
    /// via `delete <remote>/<name>` (sole deletion path; evict == prune --apply).
    pub fn prune(&self, remote: &str, apply: bool) -> Result<Vec<String>, String> {
        let target = format!("{remote}:");
        let args = ["lsjson", target.as_str()];
        let out = run(&self.bin, &args, None)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        let text = String::from_utf8_lossy(&bytes);
        let trimmed = text.trim();
        let mut names: Vec<String> = if trimmed.is_empty() {
            Vec::new()
        } else {
            let v: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| format!("parse lsjson: {e}"))?;
            let arr = v
                .as_array()
                .ok_or_else(|| "parse lsjson: expected array".to_string())?;
            let mut out = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                    continue;
                }
                let obj = item.as_object().ok_or_else(|| {
                    "parse lsjson: entries must be strings or objects".to_string()
                })?;
                let is_dir = obj
                    .get("IsDir")
                    .and_then(|v| v.as_bool())
                    .or_else(|| obj.get("isDir").and_then(|v| v.as_bool()))
                    .unwrap_or(false);
                if is_dir {
                    continue;
                }
                let name = obj
                    .get("Name")
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("name").and_then(|v| v.as_str()))
                    .or_else(|| obj.get("Path").and_then(|v| v.as_str()))
                    .or_else(|| obj.get("path").and_then(|v| v.as_str()))
                    .ok_or_else(|| "parse lsjson: object missing Name".to_string())?;
                out.push(name.to_string());
            }
            out
        };
        names.sort();
        if apply {
            for name in &names {
                let dest = format!("{remote}/{name}");
                let dargs = ["delete", dest.as_str()];
                let dout = run(&self.bin, &dargs, None)?;
                ok_or_stderr(dout, &self.bin, &dargs)?;
            }
        }
        Ok(names)
    }

    /// Local files under `root` (recursive, excluding `.innen`) not yet in
    /// `<root>/.innen/cloud-manifest.json`. Missing manifest → all local.
    pub fn candidates(&self, root: &Path) -> Result<Vec<String>, String> {
        let local = list_local_files(root)?;
        let manifest = read_manifest(root)?;
        Ok(local
            .into_iter()
            .filter(|p| !manifest.contains(p))
            .collect())
    }
}

/// Recursive file list as relative posix paths, sorted; skips `.innen` dirs.
fn list_local_files(root: &Path) -> Result<Vec<String>, String> {
    if !root.is_dir() {
        return Err(format!("root is not a directory: {}", root.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name().to_string_lossy() == ".innen" {
                    continue;
                }
                stack.push(path);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| format!("strip_prefix: {e}"))?;
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Manifest set; missing file → empty (all local are candidates).
fn read_manifest(root: &Path) -> Result<BTreeSet<String>, String> {
    let path = root.join(".innen/cloud-manifest.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let arr = if let Some(a) = v.as_array() {
        a
    } else if let Some(a) = v.get("files").and_then(|x| x.as_array()) {
        a
    } else {
        return Err(format!(
            "parse {}: expected array or {{\"files\": [...]}}",
            path.display()
        ));
    };
    let mut set = BTreeSet::new();
    for item in arr {
        match item.as_str() {
            Some(s) => {
                set.insert(s.to_string());
            }
            None => {
                return Err(format!(
                    "parse {}: manifest entries must be strings",
                    path.display()
                ));
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    /// Serializes tests that mutate process-global `FAKE_RCLONE_LOG`.
    /// Distinct temp filenames alone do not fix `set_var` races; the lock
    /// closes the set → spawn window.
    fn log_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn cloud_status_parses_stub() {
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let out = r.status("myremote").unwrap();
        assert!(out.contains("canned-dir"));
    }

    #[test]
    fn cloud_put_calls_rcat() {
        let _guard = log_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("rclone-put.log");
        std::env::set_var("FAKE_RCLONE_LOG", &log_file);
        let f = dir.path().join("a.bin");
        std::fs::write(&f, b"12345").unwrap();
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        r.put(&f, "myremote").unwrap();
        let log = std::fs::read_to_string(&log_file).unwrap();
        assert!(log.contains("rcat myremote/a.bin"));
        assert!(log.contains("bytes=5"));
    }

    #[test]
    fn cloud_get_writes_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let n = Rclone {
            bin: fixture("fake-rclone.sh"),
        }
        .get("myremote/a.bin", &dest)
        .unwrap();
        assert_eq!(n, std::fs::metadata(&dest).unwrap().len());
        assert!(n > 0);
    }

    #[test]
    fn cloud_doctor_missing_binary() {
        let r = Rclone {
            bin: std::path::PathBuf::from("/nonexistent/rclone-xyz"),
        };
        assert!(r.doctor().is_err());
    }

    #[test]
    fn cloud_prune_lists_orphans() {
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let orphans = r.prune("myremote", false).unwrap();
        assert_eq!(orphans, vec!["orphan-a.bin", "orphan-b.bin"]);
    }

    #[test]
    fn cloud_prune_apply_deletes() {
        let _guard = log_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("rclone-prune.log");
        std::env::set_var("FAKE_RCLONE_LOG", &log_file);
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let deleted = r.prune("myremote", true).unwrap();
        assert_eq!(deleted, vec!["orphan-a.bin", "orphan-b.bin"]);
        let log = std::fs::read_to_string(&log_file).unwrap();
        assert!(log.contains("delete myremote/orphan-a.bin"));
        assert!(log.contains("delete myremote/orphan-b.bin"));
    }

    #[test]
    fn cloud_candidates_missing_manifest_returns_all() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"a").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"b").unwrap();
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let got = r.candidates(dir.path()).unwrap();
        assert_eq!(got, vec!["a.bin", "b.bin"]);
    }

    #[test]
    fn cloud_candidates_filters_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"a").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"b").unwrap();
        std::fs::create_dir_all(dir.path().join(".innen")).unwrap();
        std::fs::write(
            dir.path().join(".innen/cloud-manifest.json"),
            r#"["a.bin"]"#,
        )
        .unwrap();
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let got = r.candidates(dir.path()).unwrap();
        assert_eq!(got, vec!["b.bin"]);
    }
}

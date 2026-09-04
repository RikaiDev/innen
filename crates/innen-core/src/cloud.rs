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
//! Pipe-deadlock safety: never buffer a potentially large child stdout/stderr
//! through OS pipes while polling [`std::process::Child::try_wait`]. A child
//! that fills the 64 KiB pipe buffer blocks on write while the parent waits
//! for exit → spurious timeout/deadlock. Each path avoids this as follows:
//! - `run_small` (used by `status`/`doctor`/`prune` list + `delete`): uses
//!   [`std::process::Command::output`], which drains both pipes concurrently
//!   while waiting, so no wait-then-read window. Safe because these outputs
//!   are bounded small listings / short errors (`lsd`, `version`, `lsjson`,
//!   `delete` ack) — never file bodies — so in-memory buffering cannot fill
//!   the pipe or OOM.
//! - `Rclone::get` (`run_get`): child stdout is redirected DIRECTLY to the
//!   dest file via [`std::process::Stdio::from`] (no pipe, no in-memory
//!   buffer); only stderr stays piped and `rclone cat` stderr is bounded
//!   status/error text, so buffering it after exit is safe.
//! - `Rclone::put` (`run_put`): child stdin streams DIRECTLY from the source
//!   [`std::fs::File`] handle via `Stdio::from` (no `read` + `write_all`
//!   through a pipe); stdout/stderr stay piped but `rclone rcat` emits only a
//!   small ack / short error, so in-memory buffering is safe.
//!
//! Kill reliability: on timeout we call [`std::process::Child::kill`] only
//! (no new deps for process-group kills). Limitation: grandchildren escape
//! and outlive the kill. Acceptable here because both the test stub and the
//! real `rclone cat`/`rcat`/`lsjson` invocations are leaf processes (they do
//! not daemonize grandchildren).
//!
//! Error context: every subprocess-derived `Err` includes bin + argv + exit
//! status, e.g. `rclone lsd <remote>: exit status 3: <stderr>`. There are no
//! bare-stderr returns.
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

/// Format a subprocess failure with bin + argv + exit status + stderr.
/// Never returns bare stderr.
fn status_err(
    bin: &Path,
    args: &[&str],
    status: std::process::ExitStatus,
    stderr: &[u8],
) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if stderr.is_empty() {
        format!(
            "{} {}: {status}: <empty stderr>",
            bin.display(),
            args.join(" ")
        )
    } else {
        format!("{} {}: {status}: {stderr}", bin.display(), args.join(" "))
    }
}

/// Map a finished child to `Ok(stdout bytes)` / `Err(bin + argv + status + stderr)`.
fn ok_or_stderr(
    output: std::process::Output,
    bin: &Path,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(status_err(bin, args, output.status, &output.stderr))
    }
}

/// Small-output helper for `status`/`version`/`lsjson`/`delete`.
///
/// Uses [`std::process::Command::output`], which drains stdout/stderr
/// concurrently while waiting — no try_wait-then-read deadlock window.
/// Safe to buffer in memory because these commands emit only bounded small
/// listings / acks / short errors, never file bodies.
fn run_small(bin: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    use std::process::Stdio;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.output()
        .map_err(|e| format!("{} {}: spawn: {e}", bin.display(), args.join(" ")))
}

/// Poll `child` until exit or `TIMEOUT`; `kill` on expiry.
///
/// Keeps `child.kill()` only (no process-group deps). See module docs for the
/// grandchildren-outlive limitation (acceptable: stub and rclone are leaves).
fn wait_or_kill(
    child: &mut std::process::Child,
    bin: &Path,
    args: &[&str],
) -> Result<std::process::ExitStatus, String> {
    let start = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("{} {}: wait: {e}", bin.display(), args.join(" ")))?
        {
            Some(status) => return Ok(status),
            None => {
                if start.elapsed() > TIMEOUT {
                    // Kill reliability: plain child.kill(), no process-group
                    // kill (no new deps). Grandchildren would outlive this;
                    // acceptable because stub/rclone here are leaf processes.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} {}: timeout after {}s (killed)",
                        bin.display(),
                        args.join(" "),
                        TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// `rclone cat` with stdout redirected DIRECTLY to `dest` (no pipe/buffer).
///
/// Deadlock safety: bulk bytes never touch a pipe — only stderr is piped and
/// `cat` stderr is bounded status/error text, so reading it after exit is safe.
fn run_get(bin: &Path, args: &[&str], dest: &Path) -> Result<u64, String> {
    use std::io::Read as _;
    use std::process::Stdio;
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "{} {}: create_dir {}: {e}",
                    bin.display(),
                    args.join(" "),
                    parent.display()
                )
            })?;
        }
    }
    let file = std::fs::File::create(dest).map_err(|e| {
        format!(
            "{} {}: create {}: {e}",
            bin.display(),
            args.join(" "),
            dest.display()
        )
    })?;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{} {}: spawn: {e}", bin.display(), args.join(" ")))?;
    let status = wait_or_kill(&mut child, bin, args)?;
    let mut stderr = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        err.read_to_end(&mut stderr)
            .map_err(|e| format!("{} {}: read stderr: {e}", bin.display(), args.join(" ")))?;
    }
    let _ = child.wait();
    if !status.success() {
        return Err(status_err(bin, args, status, &stderr));
    }
    let len = std::fs::metadata(dest)
        .map_err(|e| {
            format!(
                "{} {}: metadata {}: {e}",
                bin.display(),
                args.join(" "),
                dest.display()
            )
        })?
        .len();
    Ok(len)
}

/// `rclone rcat` with stdin streaming DIRECTLY from the source file.
///
/// Deadlock safety: no userspace `read` + piped `write_all` — the kernel
/// streams the file handle into the child. Stdout/stderr stay piped but
/// `rcat` emits only a small ack / short error, safe to buffer.
fn run_put(bin: &Path, args: &[&str], source: &Path) -> Result<std::process::Output, String> {
    use std::io::Read as _;
    use std::process::Stdio;
    let file = std::fs::File::open(source).map_err(|e| {
        format!(
            "{} {}: open {}: {e}",
            bin.display(),
            args.join(" "),
            source.display()
        )
    })?;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::from(file))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{} {}: spawn: {e}", bin.display(), args.join(" ")))?;
    let status = wait_or_kill(&mut child, bin, args)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout)
            .map_err(|e| format!("{} {}: read stdout: {e}", bin.display(), args.join(" ")))?;
    }
    if let Some(mut err) = child.stderr.take() {
        err.read_to_end(&mut stderr)
            .map_err(|e| format!("{} {}: read stderr: {e}", bin.display(), args.join(" ")))?;
    }
    let _ = child.wait();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Parse `lsjson` stdout into sorted remote names, skipping dirs.
///
/// `bin`/`args` are only used to add bin + argv context to parse errors.
fn parse_lsjson_names(text: &str, bin: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let ctx = format!("{} {}", bin.display(), args.join(" "));
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("{ctx}: parse lsjson: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| format!("{ctx}: parse lsjson: expected array"))?;
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            out.push(s.to_string());
            continue;
        }
        let obj = item
            .as_object()
            .ok_or_else(|| format!("{ctx}: parse lsjson: entries must be strings or objects"))?;
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
            .ok_or_else(|| format!("{ctx}: parse lsjson: object missing Name"))?;
        out.push(name.to_string());
    }
    out.sort();
    Ok(out)
}

pub struct Rclone {
    pub bin: PathBuf,
}

impl Rclone {
    /// Runs `lsd <remote>:` → `Ok(stdout)` or `Err(bin + argv + status + stderr)`.
    pub fn status(&self, remote: &str) -> Result<String, String> {
        let target = format!("{remote}:");
        let args = ["lsd", target.as_str()];
        let out = run_small(&self.bin, &args)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Runs `rcat <remote>/<filename>` with the local file streamed on stdin.
    pub fn put(&self, local: &Path, remote: &str) -> Result<String, String> {
        let name = local
            .file_name()
            .and_then(|o| o.to_str())
            .ok_or_else(|| format!("put: local has no file name: {}", local.display()))?;
        let target = format!("{remote}/{name}");
        let args = ["rcat", target.as_str()];
        let out = run_put(&self.bin, &args, local)?;
        let stdout = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    /// Runs `cat <remote-path>` → streams stdout directly to `dest`, returns len.
    pub fn get(&self, remote_path: &str, dest: &Path) -> Result<u64, String> {
        let args = ["cat", remote_path];
        run_get(&self.bin, &args, dest)
    }

    /// Binary responds to `version`, else `Err` (missing binary → spawn `Err`).
    pub fn doctor(&self) -> Result<String, String> {
        let args = ["version"];
        let out = run_small(&self.bin, &args)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// List remote orphans via `lsjson <remote>:`; `apply = true` deletes each
    /// via `delete <remote>/<name>` (sole deletion path; evict == prune --apply).
    pub fn prune(&self, remote: &str, apply: bool) -> Result<Vec<String>, String> {
        let target = format!("{remote}:");
        let args = ["lsjson", target.as_str()];
        let out = run_small(&self.bin, &args)?;
        let bytes = ok_or_stderr(out, &self.bin, &args)?;
        let text = String::from_utf8_lossy(&bytes);
        let names = parse_lsjson_names(&text, &self.bin, &args)?;
        if apply {
            for name in &names {
                let dest = format!("{remote}/{name}");
                let dargs = ["delete", dest.as_str()];
                let dout = run_small(&self.bin, &dargs)?;
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
    fn cloud_get_writes_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let n = Rclone {
            bin: fixture("fake-rclone.sh"),
        }
        .get("myremote/a.bin", &dest)
        .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(bytes, b"canned-bytes-12345");
        assert_eq!(n, bytes.len() as u64);
        assert_eq!(n, 18);
    }

    #[test]
    fn cloud_nonzero_exit_propagates_status() {
        // Stub contract: unknown argv → exit 3 with stderr.
        let bin = fixture("fake-rclone.sh");
        let args = ["definitely-unknown-xyz"];
        let out = run_small(&bin, &args).unwrap();
        assert!(!out.status.success());
        let err = ok_or_stderr(out, &bin, &args).unwrap_err();
        // Error context: bin + argv + exit status + stderr, no bare-stderr.
        assert!(
            err.contains("exit status: 3"),
            "missing exit status in: {err}"
        );
        assert!(err.contains("fake-rclone.sh"), "missing bin in: {err}");
        assert!(
            err.contains("definitely-unknown-xyz"),
            "missing argv in: {err}"
        );
        assert!(err.contains("unknown command"), "missing stderr in: {err}");
    }

    #[test]
    fn cloud_malformed_lsjson_errors() {
        let bin = fixture("fake-rclone.sh");
        let args = ["lsjson", "myremote:"];
        assert!(parse_lsjson_names("not json{{{", &bin, &args).is_err());
        assert!(parse_lsjson_names(r#"{"foo":1}"#, &bin, &args).is_err());
        assert!(parse_lsjson_names("[123]", &bin, &args).is_err());
        assert!(parse_lsjson_names(r#"[{"NoName":1}]"#, &bin, &args).is_err());
        // Parse errors carry bin + argv context.
        let err = parse_lsjson_names("not json", &bin, &args).unwrap_err();
        assert!(err.contains("fake-rclone.sh"), "missing bin: {err}");
        assert!(err.contains("lsjson"), "missing argv: {err}");
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

    #[test]
    fn cloud_candidates_manifest_files_variant() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"a").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"b").unwrap();
        std::fs::create_dir_all(dir.path().join(".innen")).unwrap();
        std::fs::write(
            dir.path().join(".innen/cloud-manifest.json"),
            r#"{"files": ["a.bin"]}"#,
        )
        .unwrap();
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let got = r.candidates(dir.path()).unwrap();
        assert_eq!(got, vec!["b.bin"]);
    }

    #[test]
    fn cloud_candidates_excludes_innen_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"a").unwrap();
        std::fs::create_dir_all(dir.path().join(".innen")).unwrap();
        std::fs::write(dir.path().join(".innen/keep.bin"), b"keep").unwrap();
        std::fs::write(dir.path().join(".innen/cloud-manifest.json"), r#"[]"#).unwrap();
        let r = Rclone {
            bin: fixture("fake-rclone.sh"),
        };
        let got = r.candidates(dir.path()).unwrap();
        assert_eq!(got, vec!["a.bin"]);
    }
}

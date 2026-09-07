//! Content-addressed artifact add (Task 11a).

pub mod ledger;

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("io: {0}")]
    Io(String),
    #[error("journal: {0}")]
    Journal(String),
}

#[derive(Debug, Clone)]
pub struct ArtifactReceipt {
    pub sha256: String,
    pub stored_path: PathBuf,
    pub bytes: u64,
}

/// Sanitized stored file name for `src`.
///
/// Extension allowlist: ASCII alphanumeric only, 1–10 chars. Anything else
/// (missing, empty, non-UTF8, control chars, punctuation like `-`/`+`,
/// overlong) maps to bare `artifact` (never `artifact.`). This makes
/// same-bytes inputs with disallowed exts converge to one file; convergence
/// for differing *allowed* exts is handled by reusing the existing stored
/// file in [`add`].
fn artifact_file_name(src: &Path) -> String {
    match src.extension().and_then(|o| o.to_str()) {
        Some(ext)
            if !ext.is_empty()
                && ext.len() <= 10
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!("artifact.{ext}")
        }
        _ => "artifact".to_string(),
    }
}

/// Atomic same-dir write: temp file + `fsync` + `rename`, mirroring
/// `journal.rs` quarantine rebuild. Avoids truncated `fs::write` leaving
/// half-written artifact/provenance on crash.
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), ArtifactError> {
    let mut tmp_os = target.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    let io = |ctx: String| ArtifactError::Io(ctx);
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| io(format!("create {}: {e}", tmp.display())))?;
        f.write_all(bytes)
            .map_err(|e| io(format!("write {}: {e}", tmp.display())))?;
        f.sync_all()
            .map_err(|e| io(format!("sync {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        io(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            target.display()
        ))
    })?;
    Ok(())
}

/// Scan `dir` for an existing `artifact*` file (excluding `provenance.json`
/// and `*.tmp`) whose bytes match, for cross-ext convergence.
fn find_reusable_artifact(dir: &Path, bytes: &[u8]) -> Result<Option<PathBuf>, ArtifactError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ArtifactError::Io(format!("read_dir {}: {e}", dir.display())))?;
    let mut hits = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|e| ArtifactError::Io(format!("read_dir {}: {e}", dir.display())))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "provenance.json" || name_str.ends_with(".tmp") {
            continue;
        }
        if !name_str.starts_with("artifact") {
            continue;
        }
        match std::fs::read(&path) {
            Ok(existing) if existing == bytes => hits.push(path),
            Ok(_) => continue,
            Err(e) => {
                return Err(ArtifactError::Io(format!("read {}: {e}", path.display())));
            }
        }
    }
    hits.sort();
    Ok(hits.into_iter().next())
}

/// Store `src` bytes content-addressed under
/// `<root>/03-output/artifacts/by-sha256/<first-2-hex>/<full-sha>/`
/// plus a `provenance.json` sidecar, and record the artifact node
/// (plus an optional `BELONGS_TO` edge) via [`crate::journal::Journal`].
///
/// Layout is deterministic in the content sha: re-adding the same bytes
/// reuses the same directory (no new artifact files), but still appends
/// fresh journal events (append-only journal, so the journal file grows —
/// callers must not assert file-count stability on `.innen/`).
/// Same bytes added via different source extensions converge to the single
/// already-stored artifact file instead of creating `artifact.<new-ext>`.
pub fn add(
    root: &Path,
    src: &Path,
    project: Option<&str>,
) -> Result<ArtifactReceipt, ArtifactError> {
    let bytes_vec = std::fs::read(src)
        .map_err(|e| ArtifactError::Io(format!("read {}: {e}", src.display())))?;
    let sha = crate::ids::sha256_hex(&bytes_vec);
    let byte_len = bytes_vec.len() as u64;
    let file_name = artifact_file_name(src);
    let dir = root
        .join("03-output/artifacts/by-sha256")
        .join(&sha[..2])
        .join(&sha);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ArtifactError::Io(format!("create_dir {}: {e}", dir.display())))?;
    let candidate = dir.join(&file_name);
    // Resolve stored path: prefer candidate on hit; otherwise reuse any
    // same-bytes artifact file already in the sha dir (cross-ext converge).
    let stored_path = match std::fs::read(&candidate) {
        Ok(existing) if existing == bytes_vec => candidate,
        Ok(_) => {
            write_atomic(&candidate, &bytes_vec)?;
            candidate
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(reused) = find_reusable_artifact(&dir, &bytes_vec)? {
                reused
            } else {
                write_atomic(&candidate, &bytes_vec)?;
                candidate
            }
        }
        Err(e) => {
            return Err(ArtifactError::Io(format!(
                "read {}: {e}",
                candidate.display()
            )));
        }
    };
    let observed_utc = crate::journal::observed_utc_now();
    let prov = serde_json::json!({
        "source_path": src.to_string_lossy(),
        "sha256": sha,
        "bytes": byte_len,
        "observed_utc": observed_utc,
    });
    let prov_text = serde_json::to_string(&prov)
        .map_err(|e| ArtifactError::Io(format!("serialize provenance for {sha}: {e}")))?;
    let prov_path = dir.join("provenance.json");
    write_atomic(&prov_path, format!("{prov_text}\n").as_bytes())?;

    let journal = crate::journal::Journal::open(root).map_err(|e| {
        ArtifactError::Journal(format!(
            "open journal at {}: {e} (artifact files already stored at {})",
            root.join(".innen").display(),
            dir.display()
        ))
    })?;
    let artifact_id = format!("artifact:{}", &sha[..8]);
    let label = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let node_payload = serde_json::json!({
        "id": artifact_id,
        "type": crate::graph::NodeType::Custom("artifact".to_string()).to_string(),
        "label": label,
        "provenance": "local file add",
    });
    journal.append("node.upsert", &node_payload).map_err(|e| {
        ArtifactError::Journal(format!(
            "append node.upsert for {artifact_id}: {e} (artifact files already stored at {})",
            stored_path.display()
        ))
    })?;
    if let Some(p) = project {
        let edge_payload = serde_json::json!({
            "from": artifact_id,
            "to": p,
            "type": crate::graph::EdgeType::BelongsTo.to_string(),
            "provenance": "local file add",
        });
        journal.append("edge.assert", &edge_payload).map_err(|e| {
            ArtifactError::Journal(format!(
                "append edge.assert for {artifact_id} -> {p}: {e} (artifact files already stored at {})",
                stored_path.display()
            ))
        })?;
    }
    Ok(ArtifactReceipt {
        sha256: sha,
        stored_path,
        bytes: byte_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk_file_count(root: &Path) -> usize {
        let mut count = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    count += 1;
                }
            }
        }
        count
    }

    fn sha_dir_for(root: &Path, sha: &str) -> PathBuf {
        root.join("03-output/artifacts/by-sha256")
            .join(&sha[..2])
            .join(sha)
    }

    #[test]
    fn artifact_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("note.txt");
        std::fs::write(&src, b"hello-bytes").unwrap(); // 11 bytes
        let r = add(dir.path(), &src, Some("p:x")).unwrap();
        assert_eq!(r.bytes, 11);
        assert_eq!(std::fs::read(&r.stored_path).unwrap(), b"hello-bytes");
        // Atomic write leaves no .tmp behind.
        let sha_dir = r.stored_path.parent().unwrap();
        assert!(
            std::fs::read_dir(sha_dir)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")),
            "no .tmp files should linger"
        );
        let prov: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(r.stored_path.parent().unwrap().join("provenance.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(prov["sha256"], serde_json::Value::String(r.sha256.clone()));
        let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
        assert!(journal.contains(&format!("artifact:{}", &r.sha256[..8])));
    }

    #[test]
    fn artifact_duplicate_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("note.txt");
        std::fs::write(&src, b"hello-bytes").unwrap();
        let r1 = add(dir.path(), &src, None).unwrap();
        let n_before = walk_file_count(dir.path());
        let r2 = add(dir.path(), &src, None).unwrap();
        assert_eq!(r1.sha256, r2.sha256);
        assert_eq!(r1.stored_path, r2.stored_path);
        assert_eq!(walk_file_count(dir.path()), n_before); // no new artifact dir
    }

    #[test]
    fn artifact_ext_allowlist() {
        // Allowed exts are preserved.
        assert_eq!(artifact_file_name(Path::new("note.txt")), "artifact.txt");
        assert_eq!(artifact_file_name(Path::new("a.TXT")), "artifact.TXT");
        assert_eq!(
            artifact_file_name(Path::new("a.abcdefghij")),
            "artifact.abcdefghij"
        );
        // Disallowed → bare `artifact` (never `artifact.`).
        assert_eq!(artifact_file_name(Path::new("note")), "artifact");
        assert_eq!(artifact_file_name(Path::new(".sh")), "artifact");
        assert_eq!(artifact_file_name(Path::new("note.")), "artifact");
        assert_eq!(artifact_file_name(Path::new("note.t-xt")), "artifact");
        assert_eq!(artifact_file_name(Path::new("note.t_xt")), "artifact");
        assert_eq!(
            artifact_file_name(Path::new("note.abcdefghijk")),
            "artifact"
        ); // 11 > 10
        assert_eq!(artifact_file_name(Path::new("note.tx t")), "artifact");
        assert_eq!(artifact_file_name(Path::new("note.c++")), "artifact");
        // Control chars rejected.
        assert_eq!(artifact_file_name(Path::new("note.tx\x01t")), "artifact");
        assert_eq!(artifact_file_name(Path::new("note.a\nb")), "artifact");
        // Non-UTF8 rejected (Unix-only: construct via bytes).
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let raw = b"note.\xff\xfe";
            let p = Path::new(OsStr::from_bytes(raw));
            assert_eq!(artifact_file_name(p), "artifact");
        }
        // End-to-end: disallowed exts store as bare `artifact`.
        let dir = tempfile::tempdir().unwrap();
        for name in ["note.t-xt", "note.abcdefghijk"] {
            let src = dir.path().join(name);
            std::fs::write(&src, format!("bytes-{name}").as_bytes()).unwrap();
            let r = add(dir.path(), &src, None).unwrap();
            assert_eq!(
                r.stored_path.file_name().unwrap().to_str().unwrap(),
                "artifact",
                "src {name} should sanitize to bare artifact"
            );
        }
    }

    #[test]
    fn artifact_same_bytes_different_ext_converges_to_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.md");
        std::fs::write(&a, b"same-bytes").unwrap();
        std::fs::write(&b, b"same-bytes").unwrap();
        let r1 = add(dir.path(), &a, None).unwrap();
        let r2 = add(dir.path(), &b, None).unwrap();
        assert_eq!(r1.sha256, r2.sha256);
        assert_eq!(r1.stored_path, r2.stored_path, "must converge to one file");
        let sha_dir = r1.stored_path.parent().unwrap();
        let mut artifact_files: Vec<_> = std::fs::read_dir(sha_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("artifact"))
                    .unwrap_or(false)
            })
            .filter(|p| p != &sha_dir.join("provenance.json"))
            .filter(|p| p.extension().map(|e| e != "tmp").unwrap_or(true))
            .collect();
        artifact_files.sort();
        assert_eq!(
            artifact_files.len(),
            1,
            "one stored file, got {artifact_files:?}"
        );
    }

    #[test]
    fn artifact_malicious_src_name_stays_under_sha_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("evil.txt");
        std::fs::write(&outside, b"evil-bytes").unwrap();
        // Path with `..` traversal that still resolves to a real file.
        let tricky = root.join("../evil.txt");
        let r = add(&root, &tricky, None).unwrap();
        let sha_dir = sha_dir_for(&root, &r.sha256);
        assert_eq!(r.stored_path.parent().unwrap(), sha_dir);
        assert!(r.stored_path.starts_with(&root));
        assert!(r.stored_path.starts_with(&sha_dir));
        assert_eq!(std::fs::read(&r.stored_path).unwrap(), b"evil-bytes");
    }

    #[test]
    fn artifact_no_ext_hidden_empty_cases() {
        let dir = tempfile::tempdir().unwrap();
        let noext = dir.path().join("note");
        std::fs::write(&noext, b"a").unwrap();
        let r = add(dir.path(), &noext, None).unwrap();
        assert_eq!(r.stored_path.file_name().unwrap(), "artifact");

        let hidden = dir.path().join(".sh");
        std::fs::write(&hidden, b"b").unwrap();
        let r2 = add(dir.path(), &hidden, None).unwrap();
        assert_eq!(r2.stored_path.file_name().unwrap(), "artifact");

        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        let r3 = add(dir.path(), &empty, None).unwrap();
        assert_eq!(r3.bytes, 0);
        assert_eq!(std::fs::read(&r3.stored_path).unwrap(), b"");
        let prov: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(r3.stored_path.parent().unwrap().join("provenance.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(prov["bytes"], serde_json::Value::from(0));
    }

    #[test]
    fn artifact_missing_src_is_io_with_context() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        let err = add(dir.path(), &missing, None).unwrap_err();
        match err {
            ArtifactError::Io(msg) => {
                assert!(msg.contains("read"), "Io must carry op: {msg}");
                assert!(
                    msg.contains("nope.txt"),
                    "Io must carry path context: {msg}"
                );
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn artifact_journal_failure_maps_to_journal_with_context() {
        // Simulation choice (documented): block `Journal::open` by making
        // `<root>/.innen` a regular file, so `create_dir_all(.innen)` fails
        // deterministically. A read-only-dir (chmod) simulation is infeasible
        // as a portable unit test: on macOS ownership/permissions vary and a
        // root-owned or permissive FS would still succeed, making the test
        // flaky. The file-as-dir trick exercises the same `Journal` mapping
        // used for every journal open/append failure (append shares the
        // identical `ArtifactError::Journal` construction with full context).
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("note.txt");
        std::fs::write(&src, b"data").unwrap();
        std::fs::write(dir.path().join(".innen"), b"block").unwrap();
        let err = add(dir.path(), &src, None).unwrap_err();
        match err {
            ArtifactError::Journal(msg) => {
                assert!(msg.contains("journal"), "must carry journal context: {msg}");
                assert!(
                    msg.contains("already stored"),
                    "must note files already stored: {msg}"
                );
            }
            other => panic!("expected Journal, got {other:?}"),
        }
        // Artifact bytes were stored before the journal failure.
        let sha = crate::ids::sha256_hex(b"data");
        let sha_dir = sha_dir_for(dir.path(), &sha);
        assert!(
            sha_dir.is_dir(),
            "artifact dir must exist despite journal failure"
        );
        let stored: Vec<_> = std::fs::read_dir(&sha_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("artifact"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(!stored.is_empty(), "artifact file must already be stored");
    }
}

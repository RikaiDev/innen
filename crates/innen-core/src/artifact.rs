//! Content-addressed artifact add (Task 11a).

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("io: {0}")]
    Io(String),
    #[error("journal already reported")]
    AlreadyReported,
}

#[derive(Debug, Clone)]
pub struct ArtifactReceipt {
    pub sha256: String,
    pub stored_path: PathBuf,
    pub bytes: u64,
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
pub fn add(
    root: &Path,
    src: &Path,
    project: Option<&str>,
) -> Result<ArtifactReceipt, ArtifactError> {
    let bytes_vec = std::fs::read(src).map_err(|e| ArtifactError::Io(e.to_string()))?;
    let sha = crate::ids::sha256_hex(&bytes_vec);
    let byte_len = bytes_vec.len() as u64;
    // Extension rule: src extension or empty → `artifact` (no ext, never `artifact.`).
    let file_name = match src
        .extension()
        .and_then(|o| o.to_str())
        .filter(|s| !s.is_empty())
    {
        Some(ext) => format!("artifact.{ext}"),
        None => "artifact".to_string(),
    };
    let dir = root
        .join("03-output/artifacts/by-sha256")
        .join(&sha[..2])
        .join(&sha);
    std::fs::create_dir_all(&dir).map_err(|e| ArtifactError::Io(e.to_string()))?;
    let stored_path = dir.join(file_name);
    let needs_write = match std::fs::read(&stored_path) {
        Ok(existing) => existing != bytes_vec,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => return Err(ArtifactError::Io(e.to_string())),
    };
    if needs_write {
        std::fs::write(&stored_path, &bytes_vec).map_err(|e| ArtifactError::Io(e.to_string()))?;
    }
    let observed_utc = crate::journal::observed_utc_now();
    let prov = serde_json::json!({
        "source_path": src.to_string_lossy(),
        "sha256": sha,
        "bytes": byte_len,
        "observed_utc": observed_utc,
    });
    let prov_text = serde_json::to_string(&prov).map_err(|e| ArtifactError::Io(e.to_string()))?;
    std::fs::write(dir.join("provenance.json"), format!("{prov_text}\n"))
        .map_err(|e| ArtifactError::Io(e.to_string()))?;

    let journal = crate::journal::Journal::open(root).map_err(|e| {
        eprintln!("error: {e}");
        ArtifactError::AlreadyReported
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
        eprintln!("error: {e}");
        ArtifactError::AlreadyReported
    })?;
    if let Some(p) = project {
        let edge_payload = serde_json::json!({
            "from": artifact_id,
            "to": p,
            "type": crate::graph::EdgeType::BelongsTo.to_string(),
            "provenance": "local file add",
        });
        journal.append("edge.assert", &edge_payload).map_err(|e| {
            eprintln!("error: {e}");
            ArtifactError::AlreadyReported
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

    #[test]
    fn artifact_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("note.txt");
        std::fs::write(&src, b"hello-bytes").unwrap(); // 11 bytes
        let r = add(dir.path(), &src, Some("p:x")).unwrap();
        assert_eq!(r.bytes, 11);
        assert_eq!(std::fs::read(&r.stored_path).unwrap(), b"hello-bytes");
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
        assert_eq!(walk_file_count(dir.path()), n_before); // no new artifact dir
    }
}

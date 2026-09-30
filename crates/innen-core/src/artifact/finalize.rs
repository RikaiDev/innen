//! Publish one final-only collection and remove an exact, hash-bound list of drafts.
//! `artifact add` remains an immutable evidence snapshot; this is the release path.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{file_sha256, ArtifactError};

const PLAN_FORMAT: &str = "innen.final-collection.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizePlan {
    pub format: String,
    pub collection: String,
    pub final_files: Vec<PlannedFile>,
    pub remove_files: Vec<PlannedFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct FinalizeResult {
    pub collection_path: PathBuf,
    pub final_files: usize,
    pub removed_files: usize,
    pub final_bytes: u64,
    pub applied: bool,
}

#[derive(Debug, Serialize)]
struct CollectionManifest {
    format: &'static str,
    collection: String,
    files: Vec<CollectionFile>,
}

#[derive(Debug, Serialize)]
struct CollectionFile {
    name: String,
    sha256: String,
    bytes: u64,
}

fn invalid(message: impl Into<String>) -> ArtifactError {
    ArtifactError::Io(message.into())
}

fn valid_collection(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn verified_path(
    output_root: &Path,
    entry: &PlannedFile,
    allow_existing_collection: bool,
) -> Result<(PathBuf, u64), ArtifactError> {
    if !entry.path.is_absolute() {
        return Err(invalid(format!(
            "path must be absolute: {}",
            entry.path.display()
        )));
    }
    if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid(format!(
            "invalid sha256 for {}",
            entry.path.display()
        )));
    }
    let metadata = fs::symlink_metadata(&entry.path)
        .map_err(|e| invalid(format!("metadata {}: {e}", entry.path.display())))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "regular file required: {}",
            entry.path.display()
        )));
    }
    let canonical = entry
        .path
        .canonicalize()
        .map_err(|e| invalid(format!("canonicalize {}: {e}", entry.path.display())))?;
    if !canonical.starts_with(output_root)
        || (!allow_existing_collection
            && canonical.starts_with(output_root.join("artifacts/collections")))
    {
        return Err(invalid(format!(
            "file must be in wiki output outside collections: {}",
            canonical.display()
        )));
    }
    let (actual, bytes) = file_sha256(&canonical)?;
    if actual != entry.sha256.to_ascii_lowercase() {
        return Err(invalid(format!("sha256 mismatch: {}", canonical.display())));
    }
    Ok((canonical, bytes))
}

fn prune_empty_parents(path: &Path, output_root: &Path) {
    let mut parent = path.parent();
    while let Some(dir) = parent {
        if dir == output_root || !dir.starts_with(output_root) {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break;
        }
        parent = dir.parent();
    }
}

/// Dry-run by default. On apply, validate every hash before writing, publish
/// the final bytes under one wiki collection, then remove only listed bytes.
pub fn finalize(
    root: &Path,
    plan_path: &Path,
    apply: bool,
) -> Result<FinalizeResult, ArtifactError> {
    let root = root
        .canonicalize()
        .map_err(|e| invalid(format!("canonicalize root {}: {e}", root.display())))?;
    let output_root = root.join("03-output");
    let output_root = output_root
        .canonicalize()
        .map_err(|e| invalid(format!("canonicalize {}: {e}", output_root.display())))?;
    let encoded = fs::read(plan_path)
        .map_err(|e| invalid(format!("read plan {}: {e}", plan_path.display())))?;
    let plan: FinalizePlan = serde_json::from_slice(&encoded)
        .map_err(|e| invalid(format!("parse plan {}: {e}", plan_path.display())))?;
    if plan.format != PLAN_FORMAT
        || !valid_collection(&plan.collection)
        || plan.final_files.is_empty()
    {
        return Err(invalid(
            "plan needs the exact format, a lowercase collection slug and at least one final file",
        ));
    }
    let collection_path = output_root
        .join("artifacts/collections")
        .join(&plan.collection);
    if collection_path.exists() {
        return Err(invalid(format!(
            "collection already exists: {}",
            collection_path.display()
        )));
    }

    let mut seen_paths = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut finals = Vec::new();
    let mut removals = Vec::new();
    let mut final_bytes = 0_u64;
    for entry in &plan.final_files {
        let (path, bytes) = verified_path(&output_root, entry, false)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("final file has no name"))?
            .to_string_lossy()
            .into_owned();
        if name == "manifest.json"
            || !seen_names.insert(name.clone())
            || !seen_paths.insert(path.clone())
        {
            return Err(invalid(format!(
                "duplicate or reserved final file: {}",
                path.display()
            )));
        }
        final_bytes += bytes;
        finals.push((path, name, entry.sha256.to_ascii_lowercase(), bytes));
    }
    for entry in &plan.remove_files {
        let (path, _) = verified_path(&output_root, entry, true)?;
        if !seen_paths.insert(path.clone()) {
            return Err(invalid(format!(
                "final/removal paths overlap or repeat: {}",
                path.display()
            )));
        }
        removals.push((path, entry.sha256.to_ascii_lowercase()));
    }
    let result = FinalizeResult {
        collection_path: collection_path.clone(),
        final_files: finals.len(),
        removed_files: finals.len() + removals.len(),
        final_bytes,
        applied: apply,
    };
    if !apply {
        return Ok(result);
    }

    let parent = collection_path
        .parent()
        .ok_or_else(|| invalid("collection has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|e| invalid(format!("create_dir {}: {e}", parent.display())))?;
    let stage = parent.join(format!(
        ".{}.staging-{}",
        plan.collection,
        std::process::id()
    ));
    fs::create_dir(&stage).map_err(|e| invalid(format!("create_dir {}: {e}", stage.display())))?;
    let published = (|| {
        let mut manifest_files = Vec::new();
        for (source, name, sha256, bytes) in &finals {
            let destination = stage.join(name);
            fs::copy(source, &destination).map_err(|e| {
                invalid(format!(
                    "copy {} -> {}: {e}",
                    source.display(),
                    destination.display()
                ))
            })?;
            let (copied_sha, _) = file_sha256(&destination)?;
            if copied_sha != *sha256 {
                return Err(invalid(format!(
                    "copied bytes differ: {}",
                    destination.display()
                )));
            }
            manifest_files.push(CollectionFile {
                name: name.clone(),
                sha256: sha256.clone(),
                bytes: *bytes,
            });
        }
        let manifest = CollectionManifest {
            format: PLAN_FORMAT,
            collection: plan.collection.clone(),
            files: manifest_files,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| invalid(format!("serialize collection manifest: {e}")))?;
        fs::write(stage.join("manifest.json"), bytes)
            .map_err(|e| invalid(format!("write collection manifest: {e}")))?;
        fs::rename(&stage, &collection_path).map_err(|e| {
            invalid(format!(
                "publish {} -> {}: {e}",
                stage.display(),
                collection_path.display()
            ))
        })?;
        Ok::<(), ArtifactError>(())
    })();
    if let Err(error) = published {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }

    // Recheck each source immediately before deletion. A changed file stays put.
    for (path, _, expected, _) in &finals {
        let (actual, _) = file_sha256(path)?;
        if actual != *expected {
            return Err(invalid(format!(
                "source changed after publish: {}",
                path.display()
            )));
        }
        fs::remove_file(path).map_err(|e| invalid(format!("remove {}: {e}", path.display())))?;
        prune_empty_parents(path, &output_root);
    }
    for (path, expected) in &removals {
        let (actual, _) = file_sha256(path)?;
        if actual != *expected {
            return Err(invalid(format!(
                "draft changed after publish: {}",
                path.display()
            )));
        }
        fs::remove_file(path).map_err(|e| invalid(format!("remove {}: {e}", path.display())))?;
        prune_empty_parents(path, &output_root);
    }
    let journal = crate::journal::Journal::open(&root)
        .map_err(|e| ArtifactError::Journal(format!("open journal after publish: {e}")))?;
    journal
        .append(
            "node.upsert",
            &serde_json::json!({
                "id": format!("collection:{}", plan.collection),
                "type": crate::graph::NodeType::Custom("artifact_collection".into()).to_string(),
                "label": plan.collection,
                "path": collection_path.to_string_lossy(),
                "provenance": "final-only artifact collection",
            }),
        )
        .map_err(|e| ArtifactError::Journal(format!("append collection node: {e}")))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hash(bytes: &[u8]) -> String {
        crate::ids::sha256_hex(bytes)
    }

    #[test]
    fn finalizes_only_verified_files_into_one_collection() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("03-output/quotations");
        fs::create_dir_all(&output).unwrap();
        let final_file = output.join("final.pdf");
        let draft = output.join("draft.pdf");
        let unrelated = output.join("other.pdf");
        fs::write(&final_file, b"final").unwrap();
        fs::write(&draft, b"draft").unwrap();
        fs::write(&unrelated, b"unrelated").unwrap();
        let plan = root.path().join("plan.json");
        fs::write(
            &plan,
            serde_json::json!({
                "format": PLAN_FORMAT, "collection": "example-quotes",
                "final_files": [{"path": final_file, "sha256": hash(b"final")}],
                "remove_files": [{"path": draft, "sha256": hash(b"draft")}]
            })
            .to_string(),
        )
        .unwrap();
        let preview = finalize(root.path(), &plan, false).unwrap();
        assert!(!preview.applied);
        assert!(final_file.exists() && draft.exists());
        let result = finalize(root.path(), &plan, true).unwrap();
        assert!(result.applied);
        assert_eq!(
            fs::read(result.collection_path.join("final.pdf")).unwrap(),
            b"final"
        );
        assert!(!final_file.exists() && !draft.exists());
        assert!(unrelated.exists());
        assert!(result.collection_path.join("manifest.json").is_file());
    }

    #[test]
    fn mismatch_prevents_publication_and_deletion() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("03-output/quotations");
        fs::create_dir_all(&output).unwrap();
        let file = output.join("final.pdf");
        fs::write(&file, b"current").unwrap();
        let plan = root.path().join("plan.json");
        fs::write(
            &plan,
            serde_json::json!({
                "format": PLAN_FORMAT, "collection": "example-quotes",
                "final_files": [{"path": file, "sha256": hash(b"old")}],
                "remove_files": []
            })
            .to_string(),
        )
        .unwrap();
        assert!(finalize(root.path(), &plan, true).is_err());
        assert!(file.exists());
        assert!(!root.path().join("03-output/artifacts/collections").exists());
    }
}

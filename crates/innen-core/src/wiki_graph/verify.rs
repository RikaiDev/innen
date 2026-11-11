//! Source reference verification for wiki pages and output markdown files.

use crate::graph::materialize;
use crate::journal::Journal;
use crate::wiki_graph::parse::{collect_markdown, parse_page, split_frontmatter, field_error};
use crate::wiki_graph::projection::{absolute_local_path, resolve_source, unwrapped_ref};
use crate::wiki_graph::types::{WikiGraphError, MANAGED_BY, WikiPage};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, serde::Serialize)]
pub struct VerifyReport {
    pub pages_scanned: u64,
    pub sources_checked: u64,
    pub broken_sources: u64,
    pub drifted_sources: u64,
    pub warnings: Vec<String>,
    pub broken_details: Vec<BrokenSource>,
    pub drift_details: Vec<DriftSource>,
}

#[derive(Debug, serde::Serialize)]
pub struct BrokenSource {
    pub wiki_path: String,
    pub source_ref: String,
    pub resolved_path: Option<String>,
    pub reason: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DriftSource {
    pub wiki_path: String,
    pub source_ref: String,
    pub source_sha256: String,
    pub expected_sha256: String,
    pub locator_kind: String,
}

pub fn verify(
    root: &Path,
    include_outputs: bool,
    check_drift: bool,
) -> Result<VerifyReport, WikiGraphError> {
    let canonical_root = fs::canonicalize(root).map_err(|source| WikiGraphError::Io {
        path: root.to_path_buf(),
        source,
    })?;

    let mut all_pages = Vec::new();
    let mut warnings = Vec::new();

    // Scan 02-wiki
    let wiki_root = canonical_root.join("02-wiki");
    if wiki_root.exists() {
        let mut paths = Vec::new();
        collect_markdown(&wiki_root, &wiki_root, 0, &mut paths, &mut warnings)?;
        paths.sort();
        for path in paths {
            match parse_page(&canonical_root, &wiki_root, &path) {
                Ok(page) => all_pages.push((page, "wiki")),
                Err(error) => {
                    let wiki_path = path
                        .strip_prefix(&wiki_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    warnings.push(format!("skipped unreadable page {wiki_path}: {error}"));
                }
            }
        }
    }

    // Optionally scan 03-output/projects
    if include_outputs {
        let output_root = canonical_root.join("03-output/projects");
        if output_root.exists() {
            let mut paths = Vec::new();
            collect_markdown(&output_root, &output_root, 0, &mut paths, &mut warnings)?;
            paths.sort();
            for path in paths {
                match parse_output_page(&canonical_root, &output_root, &path) {
                    Ok(page) => all_pages.push((page, "output")),
                    Err(error) => {
                        let rel_path = path
                            .strip_prefix(&output_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned();
                        warnings.push(format!("skipped unreadable output page {rel_path}: {error}"));
                    }
                }
            }
        }
    }

    // Load journal to get existing artifacts for drift checking
    let journal = Journal::open(&canonical_root)?;
    let entries = journal.read_all()?;
    let events: Vec<Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "op": entry.op,
                "payload": entry.payload,
                "observed_utc": entry.observed_utc,
            })
        })
        .collect();
    let graph = materialize(&events, None, true);

    let mut report = VerifyReport {
        pages_scanned: all_pages.len() as u64,
        ..Default::default()
    };

    // Build artifact index by path for drift checking
    let mut artifact_by_path: BTreeMap<String, &Value> = BTreeMap::new();
    for (id, node) in &graph.nodes {
        if node.get("type").and_then(Value::as_str) == Some("Artifact") {
            if let Some(path) = node.get("provenance").and_then(|v| v.get("path")).and_then(Value::as_str) {
                artifact_by_path.insert(path.to_string(), node);
            }
            if let Some(locator_path) = node.get("locator").and_then(|v| v.get("path")).and_then(Value::as_str) {
                artifact_by_path.insert(locator_path.to_string(), node);
            }
        }
    }

    for (page, page_type) in all_pages {
        for source_ref in &page.sources {
            report.sources_checked += 1;
            let raw = source_ref.trim();

            // Skip graph: and conversation: and URL sources
            if raw.starts_with("graph:") || raw.starts_with("conversation:") || raw.starts_with("http://") || raw.starts_with("https://") {
                continue;
            }

            let local_ref = unwrapped_ref(raw);
            let path = match absolute_local_path(&canonical_root, &local_ref) {
                Some(p) => p,
                None => {
                    report.broken_sources += 1;
                    report.broken_details.push(BrokenSource {
                        wiki_path: page.wiki_path.clone(),
                        source_ref: raw.to_string(),
                        resolved_path: None,
                        reason: "path escapes knowledge-base root".to_string(),
                    });
                    continue;
                }
            };

            if !path.exists() {
                report.broken_sources += 1;
                report.broken_details.push(BrokenSource {
                    wiki_path: page.wiki_path.clone(),
                    source_ref: raw.to_string(),
                    resolved_path: Some(path.to_string_lossy().into_owned()),
                    reason: "file does not exist".to_string(),
                });
                continue;
            }

            // Drift checking
            if check_drift {
                let source_bytes = fs::read(&path).unwrap_or_default();
                let source_sha = crate::ids::sha256_hex(&source_bytes);

                // Find matching artifact
                let mut matched_artifact: Option<&Value> = None;
                for (artifact_path, artifact_node) in &artifact_by_path {
                    if path.to_string_lossy().contains(artifact_path)
                        || artifact_path.contains(&path.to_string_lossy().to_string())
                    {
                        matched_artifact = Some(artifact_node);
                        break;
                    }
                }

                if let Some(artifact_node) = matched_artifact {
                    let expected_sha = artifact_node
                        .get("provenance")
                        .and_then(|v| v.get("sha256"))
                        .and_then(Value::as_str)
                        .or_else(|| {
                            artifact_node
                                .get("locator")
                                .and_then(|v| v.get("sha256"))
                                .and_then(Value::as_str)
                        });

                    if let Some(expected) = expected_sha {
                        if source_sha != expected {
                            report.drifted_sources += 1;
                            report.drift_details.push(DriftSource {
                                wiki_path: page.wiki_path.clone(),
                                source_ref: raw.to_string(),
                                source_sha256: source_sha,
                                expected_sha256: expected.to_string(),
                                locator_kind: "artifact".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(report)
}

fn parse_output_page(
    root: &Path,
    output_root: &Path,
    path: &Path,
) -> Result<WikiPage, WikiGraphError> {
    let file = fs::File::open(path).map_err(|source| WikiGraphError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| WikiGraphError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(field_error(path, "page", "exceeds 4 MiB bound"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|error| WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: format!("Markdown is not UTF-8: {error}"),
    })?;

    let (yaml, body) = split_frontmatter(text).ok_or_else(|| WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: "expected opening and closing `---` delimiters".to_string(),
    })?;

    let docs = yaml_rust2::YamlLoader::load_from_str(yaml).map_err(|error| WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if docs.len() != 1 || !matches!(docs.first(), Some(yaml_rust2::Yaml::Hash(_))) {
        return Err(WikiGraphError::Frontmatter {
            path: path.to_path_buf(),
            message: "frontmatter must be one YAML mapping".to_string(),
        });
    }
    let doc = &docs[0];

    let title = required_scalar(doc, "title", path)?;
    let tags = required_string_list(doc, "tags", path)?;
    let sources = required_string_list(doc, "sources", path)?;
    let related = required_string_list(doc, "related", path)?;
    let node_id = optional_scalar(doc, "node_id", path)?;
    let legacy_id = optional_scalar(doc, "id", path)?;

    let relative = path.strip_prefix(output_root).map_err(|_| {
        WikiGraphError::Identity(format!("output page escaped 03-output: {}", path.display()))
    })?;
    let wiki_path = relative.to_string_lossy().replace('\\', "/");
    let absolute = root.join("03-output/projects").join(relative);

    Ok(WikiPage {
        path: absolute,
        wiki_path,
        title,
        tags,
        sources,
        related,
        explicit_id: node_id.or(legacy_id),
        body: body.to_string(),
        sha256: crate::ids::sha256_hex(&bytes),
        id: String::new(),
    })
}

fn required_scalar(doc: &yaml_rust2::Yaml, key: &str, path: &Path) -> Result<String, WikiGraphError> {
    match &doc[key] {
        yaml_rust2::Yaml::String(value) => Ok(value.clone()),
        yaml_rust2::Yaml::Integer(value) => Ok(value.to_string()),
        yaml_rust2::Yaml::Real(value) => Ok(value.clone()),
        yaml_rust2::Yaml::Boolean(value) => Ok(value.to_string()),
        yaml_rust2::Yaml::BadValue => Err(field_error(path, key, "is missing")),
        _ => Err(field_error(path, key, "must be a scalar")),
    }
}

fn optional_scalar(doc: &yaml_rust2::Yaml, key: &str, path: &Path) -> Result<Option<String>, WikiGraphError> {
    match &doc[key] {
        yaml_rust2::Yaml::BadValue | yaml_rust2::Yaml::Null => Ok(None),
        value => {
            let s = match value {
                yaml_rust2::Yaml::String(v) => Some(v.clone()),
                yaml_rust2::Yaml::Integer(v) => Some(v.to_string()),
                yaml_rust2::Yaml::Real(v) => Some(v.clone()),
                yaml_rust2::Yaml::Boolean(v) => Some(v.to_string()),
                _ => None,
            };
            Ok(s)
        }
    }
}

fn required_string_list(doc: &yaml_rust2::Yaml, key: &str, path: &Path) -> Result<Vec<String>, WikiGraphError> {
    match &doc[key] {
        yaml_rust2::Yaml::BadValue => Err(field_error(path, key, "is missing")),
        yaml_rust2::Yaml::Null => Ok(Vec::new()),
        yaml_rust2::Yaml::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                scalar_value(value).ok_or_else(|| {
                    field_error(path, key, &format!("item {index} must be a scalar"))
                })
            })
            .collect(),
        value => scalar_value(value)
            .map(|value| vec![value])
            .ok_or_else(|| field_error(path, key, "must be a scalar or list")),
    }
}

fn scalar_value(value: &yaml_rust2::Yaml) -> Option<String> {
    match value {
        yaml_rust2::Yaml::String(value) => Some(value.clone()),
        yaml_rust2::Yaml::Integer(value) => Some(value.to_string()),
        yaml_rust2::Yaml::Real(value) => Some(value.clone()),
        yaml_rust2::Yaml::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}
use super::types::{WikiGraphError, WikiPage, MAX_WIKI_DEPTH};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

pub(super) fn collect_markdown(
    wiki_root: &Path,
    dir: &Path,
    depth: usize,
    paths: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<(), WikiGraphError> {
    if depth > MAX_WIKI_DEPTH {
        warnings.push(format!(
            "skipped directory deeper than {MAX_WIKI_DEPTH}: {}",
            dir.display()
        ));
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|source| WikiGraphError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| WikiGraphError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| WikiGraphError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            let target = fs::canonicalize(&path);
            let reason = match target {
                Ok(target) if target.starts_with(wiki_root) => "symlink",
                Ok(_) => "symlink outside canonical wiki root",
                Err(_) => "unresolvable symlink",
            };
            warnings.push(format!("skipped {reason}: {}", path.display()));
            continue;
        }
        if metadata.is_dir() {
            collect_markdown(wiki_root, &path, depth + 1, paths, warnings)?;
        } else if metadata.is_file() && path.extension().and_then(|v| v.to_str()) == Some("md") {
            paths.push(path);
        }
    }
    Ok(())
}

pub(super) fn parse_page(
    root: &Path,
    wiki_root: &Path,
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
    if !crate::tap::scan_credentials(text).is_empty() {
        return Err(field_error(
            path,
            "page",
            "credential-like content; source retained, graph not updated",
        ));
    }
    let (yaml, body) = split_frontmatter(text).ok_or_else(|| WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: "expected opening and closing `---` delimiters".to_string(),
    })?;
    let docs = YamlLoader::load_from_str(yaml).map_err(|error| WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if docs.len() != 1 || !matches!(docs.first(), Some(Yaml::Hash(_))) {
        return Err(WikiGraphError::Frontmatter {
            path: path.to_path_buf(),
            message: "frontmatter must be one YAML mapping".to_string(),
        });
    }
    let doc = &docs[0];
    let title = required_scalar(doc, "title", path)?;
    if title.trim().is_empty() {
        return Err(WikiGraphError::Frontmatter {
            path: path.to_path_buf(),
            message: "`title` must not be empty".to_string(),
        });
    }
    let tags = required_string_list(doc, "tags", path)?;
    let sources = required_string_list(doc, "sources", path)?;
    let related = required_string_list(doc, "related", path)?;
    let node_id = optional_scalar(doc, "node_id", path)?;
    let legacy_id = optional_scalar(doc, "id", path)?;
    if node_id.is_some() && legacy_id.is_some() && node_id != legacy_id {
        return Err(WikiGraphError::Frontmatter {
            path: path.to_path_buf(),
            message: "`node_id` and `id` disagree".to_string(),
        });
    }
    let explicit_id = node_id.or(legacy_id);
    if explicit_id.as_deref().is_some_and(|id| !safe_node_id(id)) {
        return Err(WikiGraphError::Frontmatter {
            path: path.to_path_buf(),
            message: "explicit `node_id`/`id` contains unsafe characters".to_string(),
        });
    }
    let relative = path.strip_prefix(wiki_root).map_err(|_| {
        WikiGraphError::Identity(format!("wiki page escaped 02-wiki: {}", path.display()))
    })?;
    let wiki_path = relative.to_string_lossy().replace('\\', "/");
    let absolute = root.join("02-wiki").join(relative);
    Ok(WikiPage {
        path: absolute,
        wiki_path,
        title,
        tags,
        sources,
        related,
        explicit_id,
        body: body.to_string(),
        sha256: crate::ids::sha256_hex(&bytes),
        id: String::new(),
    })
}

fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let normalized_start = text.strip_prefix("\u{feff}").unwrap_or(text);
    let after_open = normalized_start
        .strip_prefix("---\n")
        .or_else(|| normalized_start.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        let clean = line.trim_end_matches(['\r', '\n']);
        if clean == "---" {
            let body_start = offset + line.len();
            return Some((&after_open[..offset], &after_open[body_start..]));
        }
        offset += line.len();
    }
    None
}

fn required_scalar(doc: &Yaml, key: &str, path: &Path) -> Result<String, WikiGraphError> {
    match &doc[key] {
        Yaml::String(value) => Ok(value.clone()),
        Yaml::Integer(value) => Ok(value.to_string()),
        Yaml::Real(value) => Ok(value.clone()),
        Yaml::Boolean(value) => Ok(value.to_string()),
        Yaml::BadValue => Err(field_error(path, key, "is missing")),
        _ => Err(field_error(path, key, "must be a scalar")),
    }
}

fn optional_scalar(doc: &Yaml, key: &str, path: &Path) -> Result<Option<String>, WikiGraphError> {
    match &doc[key] {
        Yaml::BadValue | Yaml::Null => Ok(None),
        value => scalar_value(value)
            .map(Some)
            .ok_or_else(|| field_error(path, key, "must be a scalar")),
    }
}

fn required_string_list(doc: &Yaml, key: &str, path: &Path) -> Result<Vec<String>, WikiGraphError> {
    match &doc[key] {
        Yaml::BadValue => Err(field_error(path, key, "is missing")),
        Yaml::Null => Ok(Vec::new()),
        Yaml::Array(values) => values
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

fn scalar_value(value: &Yaml) -> Option<String> {
    match value {
        Yaml::String(value) => Some(value.clone()),
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::Real(value) => Some(value.clone()),
        Yaml::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn field_error(path: &Path, key: &str, reason: &str) -> WikiGraphError {
    WikiGraphError::Frontmatter {
        path: path.to_path_buf(),
        message: format!("`{key}` {reason}"),
    }
}

fn safe_node_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 512
        && id == id.trim()
        && id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.' | b'/')
        })
}

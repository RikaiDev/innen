use super::identity::source_node;
use super::types::{DesiredEdge, EdgeKey, WikiPage, EDGE_PROVENANCE_PREFIX};
use crate::graph::Materialized;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

pub(super) enum Resolution {
    One(String),
    Missing,
    Ambiguous(Vec<String>),
}

pub(super) struct WikiResolver {
    by_path: HashMap<String, Vec<String>>,
    by_basename: HashMap<String, Vec<String>>,
}

impl WikiResolver {
    pub(super) fn new(pages: &[WikiPage]) -> Self {
        let mut resolver = Self {
            by_path: HashMap::new(),
            by_basename: HashMap::new(),
        };
        for page in pages {
            let stem = page
                .wiki_path
                .strip_suffix(".md")
                .unwrap_or(&page.wiki_path);
            resolver
                .by_path
                .entry(stem.to_string())
                .or_default()
                .push(page.id.clone());
            if let Some(base) = stem.rsplit('/').next() {
                resolver
                    .by_basename
                    .entry(base.to_string())
                    .or_default()
                    .push(page.id.clone());
            }
        }
        resolver
    }

    pub(super) fn resolve(&self, reference: &str) -> Resolution {
        let key = wiki_ref_key(reference);
        if let Some(ids) = self.by_path.get(&key) {
            return unique(ids);
        }
        self.by_basename
            .get(&key)
            .map_or(Resolution::Missing, |ids| unique(ids))
    }
}

fn unique(ids: &[String]) -> Resolution {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    match ids.len() {
        0 => Resolution::Missing,
        1 => Resolution::One(ids.remove(0)),
        _ => Resolution::Ambiguous(ids),
    }
}

fn wiki_ref_key(reference: &str) -> String {
    let mut value = reference.trim();
    if let Some(inner) = value.strip_prefix("[[").and_then(|v| v.strip_suffix("]]")) {
        value = inner;
    }
    value = value.split('|').next().unwrap_or(value);
    value = value.split('#').next().unwrap_or(value);
    let value = value.trim().replace('\\', "/");
    value
        .trim_start_matches("./")
        .trim_start_matches("02-wiki/")
        .strip_suffix(".md")
        .unwrap_or_else(|| {
            value
                .trim_start_matches("./")
                .trim_start_matches("02-wiki/")
        })
        .to_string()
}

pub(super) fn resolve_source(
    root: &Path,
    source_ref: &str,
    graph: &Materialized,
    resolver: &WikiResolver,
    desired_nodes: &mut BTreeMap<String, Value>,
    warnings: &mut Vec<String>,
    wiki_path: &str,
) -> Option<String> {
    let raw = source_ref.trim();
    if let Some(id) = raw.strip_prefix("graph:") {
        if graph.nodes.contains_key(id) || desired_nodes.contains_key(id) {
            return Some(id.to_string());
        }
        warnings.push(format!(
            "{wiki_path}: missing explicit graph source {raw:?}"
        ));
        return None;
    }
    if let Some(node) = graph.nodes.get(raw).or_else(|| desired_nodes.get(raw)) {
        if node.get("missing").and_then(Value::as_bool) == Some(true)
            || node.get("wiki_missing").and_then(Value::as_bool) == Some(true)
        {
            warnings.push(format!(
                "{wiki_path}: missing explicit graph source {raw:?}"
            ));
            return None;
        }
        return Some(raw.to_string());
    }
    match resolver.resolve(raw) {
        Resolution::One(id) => return Some(id),
        Resolution::Ambiguous(ids) => {
            warnings.push(format!(
                "{wiki_path}: ambiguous wiki source {raw:?}: {}",
                ids.join(", ")
            ));
            return None;
        }
        Resolution::Missing => {}
    }

    let locator = if let Some((source, session_id)) = conversation_ref(raw) {
        json!({"kind": "conversation", "source": source, "session_id": session_id})
    } else if raw.starts_with("http://") || raw.starts_with("https://") {
        json!({"kind": "url", "url": raw})
    } else {
        let local_ref = unwrapped_ref(raw);
        let path = match absolute_local_path(root, &local_ref) {
            Some(path) => path,
            None => {
                warnings.push(format!(
                    "{wiki_path}: skipped source path escaping the knowledge-base root: {raw:?}"
                ));
                return None;
            }
        };
        json!({"kind": "file", "path": path.to_string_lossy()})
    };
    let (id, node) = source_node(raw, locator);
    desired_nodes.entry(id.clone()).or_insert(node);
    Some(id)
}

fn conversation_ref(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.splitn(3, ':');
    if parts.next()? != "conversation" {
        return None;
    }
    let source = parts.next()?;
    let session_id = parts.next()?;
    (!source.is_empty() && !session_id.is_empty()).then_some((source, session_id))
}

fn unwrapped_ref(reference: &str) -> String {
    let mut value = reference.trim();
    if let Some(inner) = value.strip_prefix("[[").and_then(|v| v.strip_suffix("]]")) {
        value = inner;
    }
    value = value.split('|').next().unwrap_or(value);
    value = value.split('#').next().unwrap_or(value);
    value.trim().replace('\\', "/")
}

fn absolute_local_path(root: &Path, reference: &str) -> Option<PathBuf> {
    if let Some(relative) = reference.strip_prefix("~/") {
        let home = std::env::var_os("HOME")?;
        return Some(lexical_normalize(&PathBuf::from(home).join(relative)));
    }
    let path = Path::new(reference);
    if path.is_absolute() {
        return Some(lexical_normalize(path));
    }
    let joined = lexical_normalize(&root.join(path));
    joined.starts_with(root).then_some(joined)
}

pub(super) fn insert_edge(
    edges: &mut BTreeMap<EdgeKey, DesiredEdge>,
    from: &str,
    kind: &str,
    to: &str,
    field: &str,
    reference: &str,
) {
    let key = EdgeKey {
        from: from.to_string(),
        kind: kind.to_string(),
        to: to.to_string(),
    };
    let provenance = format!(
        "{EDGE_PROVENANCE_PREFIX}{from}:{field}:{}",
        crate::ids::sha256_hex(reference.as_bytes())
    );
    edges
        .entry(key.clone())
        .or_insert(DesiredEdge { key, provenance });
}

pub(super) fn live_owned_edges(graph: &Materialized) -> BTreeMap<EdgeKey, BTreeSet<String>> {
    let mut edges = BTreeMap::new();
    for edge in graph.edges.iter().filter(|edge| !edge.retracted) {
        let Some(provenance) = edge
            .provenance
            .as_deref()
            .filter(|v| v.starts_with(EDGE_PROVENANCE_PREFIX))
        else {
            continue;
        };
        edges
            .entry(EdgeKey {
                from: edge.from.clone(),
                kind: edge.edge.to_string(),
                to: edge.to.clone(),
            })
            .or_insert_with(BTreeSet::new)
            .insert(provenance.to_string());
    }
    edges
}

pub(super) fn live_foreign_keys(graph: &Materialized) -> BTreeSet<EdgeKey> {
    graph
        .edges
        .iter()
        .filter(|edge| {
            !edge.retracted
                && edge
                    .provenance
                    .as_deref()
                    .is_none_or(|v| !v.starts_with(EDGE_PROVENANCE_PREFIX))
        })
        .map(|edge| EdgeKey {
            from: edge.from.clone(),
            kind: edge.edge.to_string(),
            to: edge.to.clone(),
        })
        .collect()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

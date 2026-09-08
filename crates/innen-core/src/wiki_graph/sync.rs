use super::identity::{assign_ids, changed_fields, wiki_node};
use super::parse::{collect_markdown, parse_page};
use super::projection::{
    insert_edge, live_foreign_keys, live_owned_edges, resolve_source, Resolution, WikiResolver,
};
use super::types::{DesiredEdge, EdgeKey, SyncReport, WikiGraphError, MANAGED_BY};
use crate::graph::materialize;
use crate::journal::Journal;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::Path;

pub fn sync(root: &Path) -> Result<SyncReport, WikiGraphError> {
    let canonical_root = fs::canonicalize(root).map_err(|source| WikiGraphError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let wiki_root = canonical_root.join("02-wiki");
    let mut warnings = Vec::new();
    let mut paths = Vec::new();
    match fs::symlink_metadata(&wiki_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => warnings.push(format!(
            "skipped symlink used as canonical wiki root: {}",
            wiki_root.display()
        )),
        Ok(metadata) if metadata.is_dir() => {
            collect_markdown(&wiki_root, &wiki_root, 0, &mut paths, &mut warnings)?;
        }
        Ok(_) => warnings.push(format!(
            "skipped non-directory wiki root: {}",
            wiki_root.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(WikiGraphError::Io {
                path: wiki_root,
                source,
            });
        }
    }
    paths.sort();

    // This whole pass is deliberately before Journal::open.
    let mut pages = Vec::with_capacity(paths.len());
    for path in paths {
        pages.push(parse_page(&canonical_root, &wiki_root, &path)?);
    }

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

    assign_ids(&mut pages, &graph)?;
    let resolver = WikiResolver::new(&pages);
    let mut report = SyncReport {
        pages_scanned: pages.len() as u64,
        warnings,
        ..SyncReport::default()
    };

    let mut desired_nodes: BTreeMap<String, Value> = BTreeMap::new();
    let present_paths: HashSet<&str> = pages.iter().map(|page| page.wiki_path.as_str()).collect();
    for page in &pages {
        desired_nodes.insert(page.id.clone(), wiki_node(page));
    }

    let mut desired_edges: BTreeMap<EdgeKey, DesiredEdge> = BTreeMap::new();
    for page in &pages {
        for source_ref in &page.sources {
            if let Some(target) = resolve_source(
                &canonical_root,
                source_ref,
                &graph,
                &resolver,
                &mut desired_nodes,
                &mut report.warnings,
                &page.wiki_path,
            ) {
                insert_edge(
                    &mut desired_edges,
                    &page.id,
                    "DERIVED_FROM",
                    &target,
                    "sources",
                    source_ref,
                );
            }
        }
        for related_ref in &page.related {
            match resolver.resolve(related_ref) {
                Resolution::One(target) => insert_edge(
                    &mut desired_edges,
                    &page.id,
                    "RELATED_TO",
                    &target,
                    "related",
                    related_ref,
                ),
                Resolution::Missing => report.warnings.push(format!(
                    "{}: missing related wiki reference {:?}",
                    page.wiki_path, related_ref
                )),
                Resolution::Ambiguous(ids) => report.warnings.push(format!(
                    "{}: ambiguous related wiki reference {:?}: {}",
                    page.wiki_path,
                    related_ref,
                    ids.join(", ")
                )),
            }
        }
    }

    let mut node_appends = Vec::new();
    for (id, desired) in &desired_nodes {
        let current = graph.nodes.get(id);
        if let Some(existing) = current {
            let owner = existing.get("managed_by").and_then(Value::as_str);
            if owner != Some(MANAGED_BY) {
                return Err(WikiGraphError::Identity(format!(
                    "projection id {id:?} already belongs to an unmanaged node"
                )));
            }
            if existing.get("type") != desired.get("type") {
                return Err(WikiGraphError::Identity(format!(
                    "projection id {id:?} is already registered as type {:?}, not {:?}",
                    existing.get("type"),
                    desired.get("type")
                )));
            }
        }
        if let Some(payload) = changed_fields(current, desired) {
            let is_wiki = desired.get("type") == Some(&json!("Wiki"));
            match (current.is_some(), is_wiki) {
                (false, true) => report.wiki_nodes_created += 1,
                (true, true) => report.wiki_nodes_updated += 1,
                (false, false) => report.source_nodes_created += 1,
                (true, false) => report.source_nodes_updated += 1,
            }
            node_appends.push(payload);
        }
    }

    // Deleted pages retain their node and path registration, with a new
    // missing-state event. No history is erased.
    for (id, node) in &graph.nodes {
        if node.get("managed_by").and_then(Value::as_str) != Some(MANAGED_BY)
            || node.get("type").and_then(Value::as_str) != Some("Wiki")
        {
            continue;
        }
        let Some(path) = node.get("wiki_path").and_then(Value::as_str) else {
            continue;
        };
        if !present_paths.contains(path) && node.get("missing") != Some(&Value::Bool(true)) {
            node_appends.push(json!({"id": id, "missing": true}));
            report.wiki_nodes_marked_missing += 1;
        }
    }

    let live_owned = live_owned_edges(&graph);
    let live_foreign = live_foreign_keys(&graph);
    let mut retracts = BTreeSet::new();
    let mut asserts = Vec::new();

    for (key, provenances) in &live_owned {
        let desired = desired_edges.get(key);
        let has_exact = desired.is_some_and(|edge| provenances.contains(edge.provenance.as_str()));
        let has_stale = desired.is_some_and(|edge| {
            provenances
                .iter()
                .any(|provenance| provenance.as_str() != edge.provenance.as_str())
        });
        if desired.is_none() || !has_exact || has_stale {
            if live_foreign.contains(key) {
                report.warnings.push(format!(
                    "preserved shared edge {} -[{}]-> {}; foreign assertion prevents owned-row retraction",
                    key.from, key.kind, key.to
                ));
            } else {
                retracts.insert(key.clone());
            }
        }
    }
    for (key, edge) in &desired_edges {
        let exact_live = live_owned
            .get(key)
            .is_some_and(|values| values.contains(edge.provenance.as_str()));
        if !exact_live || retracts.contains(key) {
            asserts.push(edge.clone());
        }
    }

    for payload in node_appends {
        journal.append("node.upsert", &payload)?;
    }
    for key in retracts {
        journal.append(
            "edge.retract",
            &json!({"from": key.from, "type": key.kind, "to": key.to}),
        )?;
        report.edges_retracted += 1;
    }
    for edge in asserts {
        journal.append(
            "edge.assert",
            &json!({
                "from": edge.key.from,
                "type": edge.key.kind,
                "to": edge.key.to,
                "managed_by": MANAGED_BY,
                "provenance": edge.provenance,
            }),
        )?;
        report.edges_asserted += 1;
    }
    report.warnings.sort();
    report.warnings.dedup();
    Ok(report)
}

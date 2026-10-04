//! Project page render: members grouped by kind under fixed headings.

use std::path::Path;

use crate::graph::materialize;
use crate::journal::Journal;

/// Groups `BELONGS_TO` members whose `to` == `id` by member node `type`
/// (case-insensitive `decision`/`task`/`experiment`/`dataset`; other kinds
/// ignored) and lists member labels as `- ` bullets under `## Decisions` /
/// `## Tasks` / `## Experiments` / `## Datasets` (always emitted, in that
/// order, members sorted for determinism; missing/empty label falls back to
/// the member id). Retracted edges are skipped; validity windows are ignored
/// (`materialize(..., include_expired = true)`, same as `status`/`timeline`).
/// Unknown id (no node with that id, including unreadable journal) errors.
pub fn project_render(root: &Path, id: &str) -> Result<String, String> {
    let materialized = match Journal::open(root).and_then(|j| {
        j.read_all().map(|entries| {
            entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "op": e.op,
                        "payload": e.payload,
                        "observed_utc": e.observed_utc,
                    })
                })
                .collect::<Vec<_>>()
        })
    }) {
        Ok(events) => materialize(&events, None, true),
        Err(_) => materialize(&[], None, true),
    };
    let resolved = crate::graph::resolve_project_id(&materialized, id)?;
    let (target_id, project) = materialized
        .nodes
        .get_key_value(&resolved)
        .map(|(key, value)| (key.as_str(), value))
        .expect("resolved project exists");
    let project_label = project
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(target_id);
    let mut decisions = Vec::new();
    let mut tasks = Vec::new();
    let mut experiments = Vec::new();
    let mut datasets = Vec::new();
    for edge in &materialized.edges {
        if edge.retracted || edge.to != target_id || edge.edge != crate::graph::EdgeType::BelongsTo
        {
            continue;
        }
        let Some(node) = materialized.nodes.get(&edge.from) else {
            continue;
        };
        let label = node
            .get("label")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(edge.from.as_str())
            .to_string();
        match node
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "decision" => decisions.push(label),
            "task" => tasks.push(label),
            "experiment" => experiments.push(label),
            "dataset" => datasets.push(label),
            _ => {}
        }
    }
    decisions.sort();
    tasks.sort();
    experiments.sort();
    datasets.sort();
    let mut out = format!("# {project_label}\n\n");
    let sections = [
        ("## Decisions", &decisions),
        ("## Tasks", &tasks),
        ("## Experiments", &experiments),
        ("## Datasets", &datasets),
    ];
    for (i, (header, members)) in sections.iter().enumerate() {
        out.push_str(header);
        out.push('\n');
        for m in members.iter() {
            out.push_str("- ");
            out.push_str(m);
            out.push('\n');
        }
        if i + 1 < sections.len() {
            out.push('\n');
        }
    }
    Ok(out)
}

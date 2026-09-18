//! Task navigation over canonical replay. No transcript heuristics or status inference.
use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::graph::{materialize, EdgeType};
use crate::journal::Journal;

pub fn is_terminal(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "complete" | "completed" | "done" | "closed" | "cancelled" | "canceled" | "superseded"
    )
}

#[derive(Default)]
pub struct TaskOptions<'a> {
    pub project: Option<&'a str>,
    pub all: bool,
    pub detail: bool,
    pub offset: usize,
    pub limit: usize,
}

pub fn list(root: &Path, options: &TaskOptions<'_>) -> Result<Value, String> {
    // Unlike Journal::open, a read of a nonexistent KB must not create one.
    if !root.join(".innen/journal.jsonl").is_file() {
        return Err(options
            .project
            .map(|id| format!("unknown project: {id}"))
            .unwrap_or_else(|| {
                "knowledge journal unavailable; supply --root <knowledge-base>".into()
            }));
    }
    let entries = Journal::open(root)
        .and_then(|j| j.read_all())
        .map_err(|e| e.to_string())?;
    let harvest_report = crate::harvest::check(root);
    let (harvest_pending, harvest_skipped) = harvest_report
        .taps
        .iter()
        .fold((0usize, 0usize), |(pending, skipped), tap| {
            (pending + tap.new_files.len(), skipped + tap.skipped.len())
        });
    let events: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "op": e.op, "payload": e.payload, "observed_utc": e.observed_utc
            })
        })
        .collect();
    let now = crate::journal::observed_utc_now();
    let graph = materialize(&events, Some(&now), false);
    let project = options
        .project
        .map(|input| crate::graph::resolve_project_id(&graph, input))
        .transpose()?;
    let mut tasks = Vec::new();
    let mut excluded = 0;
    for (id, node) in &graph.nodes {
        if !text(node, "type").eq_ignore_ascii_case("task") {
            continue;
        }
        let projects: BTreeSet<&str> = graph
            .edges
            .iter()
            .filter(|e| !e.retracted && e.from == *id && e.edge == EdgeType::BelongsTo)
            .filter(|e| {
                graph
                    .nodes
                    .get(&e.to)
                    .is_some_and(|n| text(n, "type").eq_ignore_ascii_case("project"))
            })
            .map(|e| e.to.as_str())
            .collect();
        if project
            .as_ref()
            .is_some_and(|p| !projects.contains(p.as_str()))
        {
            continue;
        }
        let status = text(node, "status");
        if !options.all && is_terminal(status) {
            excluded += 1;
            continue;
        }
        let status = if status.is_empty() { "unknown" } else { status };
        let label = text(node, "label");
        tasks.push(json!({"id": id, "projects": projects, "status": status,
            "label": if label.is_empty() {id} else {label},
            "updated": text(node,"observed_utc"), "next_action": node.get("next_action").cloned().unwrap_or(Value::Null),
            "blockers": node.get("blockers").or_else(|| node.get("blocker")).cloned().unwrap_or(Value::Null)}));
    }
    tasks.sort_by(|a, b| {
        a["projects"]
            .to_string()
            .cmp(&b["projects"].to_string())
            .then_with(|| text(a, "id").cmp(text(b, "id")))
    });
    let total = tasks.len();
    let selected: Vec<_> = tasks
        .into_iter()
        .skip(options.offset)
        .take(options.limit)
        .collect();
    let next = options.offset.saturating_add(selected.len());
    let mut out = json!({"scope": "recorded tasks; unknown status is not proof of unfinished work",
    "total":total,"excluded_terminal":excluded,"next_offset":if next < total {Some(next)} else {None},
    "detail":"innen project <project-id> --view evidence",
    "harvest": {
        "pending": harvest_pending,
        "skipped": harvest_skipped,
        "check_command": "innen --format json harvest --check",
        "ingest_command": "innen --format json ingest"
    }});
    let projects: std::collections::BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter(|(id, n)| {
            text(n, "type").eq_ignore_ascii_case("project")
                && project.as_ref().is_none_or(|p| p == *id)
        })
        .map(|(id, n)| (id.clone(), text(n, "label").to_string()))
        .collect();
    out["projects"] = json!(projects);
    if options.detail {
        // Evidence view is an intentionally bounded closure: project direct
        // links plus task links, and DESCRIBES links from those tasks.  The
        // compact brief above remains unchanged.
        if let Some(project_id) = project.as_deref() {
            let task_ids: BTreeSet<String> = graph
                .nodes
                .iter()
                .filter(|(id, node)| {
                    text(node, "type").eq_ignore_ascii_case("task")
                        && graph.edges.iter().any(|e| {
                            !e.retracted
                                && e.from == **id
                                && e.to == project_id
                                && e.edge == EdgeType::BelongsTo
                        })
                })
                .map(|(id, _)| id.clone())
                .collect();
            let mut ids = BTreeSet::from([project_id.to_string()]);
            let mut chosen_edges = Vec::new();
            for edge in graph.edges.iter().filter(|e| !e.retracted) {
                let direct = (edge.from == project_id || edge.to == project_id)
                    || task_ids.contains(&edge.from)
                    || task_ids.contains(&edge.to);
                let describes_task_source = edge.edge.to_string() == "DESCRIBES"
                    && (task_ids.contains(&edge.from) || task_ids.contains(&edge.to));
                if !(direct || describes_task_source) || chosen_edges.len() >= 256 {
                    continue;
                }
                let Some(from) = graph.nodes.get(&edge.from) else {
                    continue;
                };
                let Some(to) = graph.nodes.get(&edge.to) else {
                    continue;
                };
                let allowed = |n: &Value| {
                    matches!(
                        text(n, "type").to_ascii_lowercase().as_str(),
                        "artifact" | "conversation" | "source"
                    )
                };
                let endpoint_ok =
                    |id: &str, n: &Value| id == project_id || task_ids.contains(id) || allowed(n);
                if endpoint_ok(&edge.from, from) && endpoint_ok(&edge.to, to) {
                    ids.insert(edge.from.clone());
                    ids.insert(edge.to.clone());
                    chosen_edges.push(json!({"from":edge.from,"to":edge.to,"type":edge.edge.to_string(),"provenance":edge.provenance,"observed_utc":edge.observed_utc}));
                }
            }
            let nodes: Vec<_> = ids
                .into_iter()
                .take(128)
                .filter_map(|id| graph.nodes.get(&id).map(|n| json!({"id":id,"node":n})))
                .collect();
            out["evidence_nodes"] = json!(nodes);
            out["evidence_edges"] = json!(chosen_edges);
            out["evidence_boundary"] = json!("bounded project/task provenance closure; edges are evidence routes, not ownership proof");
        }
        out["tasks"] = Value::Array(selected.into_iter().map(|mut task| {
            let id = text(&task,"id").to_string();
            task["node"] = graph.nodes[&id].clone();
            task["history"] = Value::Array(entries.iter().enumerate().filter(|(_,e)|
                (e.op == "node.upsert" && text(&e.payload,"id") == id) ||
                (matches!(e.op.as_str(),"edge.assert"|"edge.retract") &&
                 (text(&e.payload,"from") == id || text(&e.payload,"to") == id)))
                .map(|(i,e)| json!({"line":i+1,"event":e.id,"at":e.observed_utc,"op":e.op,"payload":e.payload})).collect());
            task
        }).collect());
    } else {
        out["fields"] = json!([
            "id",
            "projects",
            "status",
            "label",
            "updated",
            "next_action",
            "blockers"
        ]);
        out["rows"] = Value::Array(
            selected
                .iter()
                .map(|t| {
                    json!([
                        t["id"],
                        t["projects"],
                        t["status"],
                        t["label"],
                        t["updated"],
                        t["next_action"],
                        t["blockers"]
                    ])
                })
                .collect(),
        );
    }
    Ok(out)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

pub fn render(value: &Value) -> String {
    if value.get("rows").is_none() {
        return serde_json::to_string_pretty(value).unwrap() + "\n";
    }
    let mut out = format!(
        "Recorded tasks: {} (terminal excluded: {}); next_offset={}\n",
        value["total"], value["excluded_terminal"], value["next_offset"]
    );
    let pending = value["harvest"]["pending"].as_u64().unwrap_or(0);
    let skipped = value["harvest"]["skipped"].as_u64().unwrap_or(0);
    if pending > 0 || skipped > 0 {
        out.push_str(&format!(
            "Harvest pending: {pending}; skipped: {skipped}; run `innen --format json ingest` after reviewing `innen --format json harvest --check`\n"
        ));
    }
    if let Some(rows) = value["rows"].as_array() {
        for row in rows {
            out.push_str(&format!(
                "{} | [{}] {} | {} | {}\n",
                row[1],
                row[2].as_str().unwrap_or(""),
                row[3].as_str().unwrap_or(""),
                row[0].as_str().unwrap_or(""),
                row[4].as_str().unwrap_or("")
            ));
            if !row[5].is_null() {
                out.push_str(&format!("  Next: {}\n", row[5]));
            }
            if !row[6].is_null() {
                out.push_str(&format!("  Blockers: {}\n", row[6]));
            }
        }
    }
    out.push_str(
        "Unknown status is unverified. Evidence: innen project <project-id> --view evidence\n",
    );
    out
}

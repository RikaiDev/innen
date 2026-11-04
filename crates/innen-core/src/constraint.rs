//! Record a user's binding requirement as a `Constraint` node owned by a
//! project. The text is stored as given and the source is mandatory: a
//! requirement nobody can trace back to its source cannot be checked.

use std::path::Path;

use serde_json::{json, Value};

use crate::edge_write::{append_edge_assert, EdgeAssert};
use crate::graph::{materialize, EdgeType};
use crate::ids::sha256_hex;
use crate::journal::{observed_utc_now, Journal};

const LABEL_CHARS: usize = 40;

/// Append the constraint node and its `BELONGS_TO` edge; returns the node id.
/// Without `id` the id derives from the text, so recording the same words twice
/// addresses one node.
pub fn add_constraint(
    root: &Path,
    project: &str,
    source: &str,
    text: &str,
    id: Option<&str>,
    label: Option<&str>,
) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("constraint text is empty".into());
    }
    if source.trim().is_empty() {
        return Err("constraint needs --source: text without a source cannot be checked".into());
    }
    let journal = Journal::open(root).map_err(|e| e.to_string())?;
    let events: Vec<Value> = journal
        .read_all()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|e| json!({"op": e.op, "payload": e.payload, "observed_utc": e.observed_utc}))
        .collect();
    let graph = materialize(&events, Some(&observed_utc_now()), false);
    let is_project = graph.nodes.get(project).is_some_and(|n| {
        n.get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.eq_ignore_ascii_case("project"))
    });
    if !is_project {
        return Err(format!("project node not found: {project}"));
    }
    let id = id.map(str::to_owned).unwrap_or_else(|| {
        let short = &sha256_hex(text.as_bytes())[..8];
        let name = project.strip_prefix("project:").unwrap_or(project);
        format!("constraint:{name}:{short}")
    });
    let label = label
        .map(str::to_owned)
        .unwrap_or_else(|| text.chars().take(LABEL_CHARS).collect());
    journal
        .append(
            "node.upsert",
            &json!({"id": id, "type": "Constraint", "label": label, "body": text, "provenance": source}),
        )
        .map_err(|e| e.to_string())?;
    append_edge_assert(
        root,
        &EdgeAssert {
            from: &id,
            edge: EdgeType::BelongsTo,
            to: project,
            weight: None,
            valid_from: None,
            valid_until: None,
            provenance: Some(source),
            allow_dangling: false,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Record user turns of a session verbatim. Every line must be a user turn,
/// and a quotation must occur word for word in its turn, so the stored text is
/// what the user wrote; nothing is written unless every line checks out.
pub fn add_from_session(
    root: &Path,
    source_root: Option<&Path>,
    source: &str,
    project: &str,
    session: &str,
    lines: &[usize],
    quote: Option<&str>,
) -> Result<Vec<String>, String> {
    if quote.is_some() && lines.len() != 1 {
        return Err("--quote applies to exactly one --line".into());
    }
    let found = crate::constraint_turns::user_turns(source_root, source, session)?;
    let mut picked: Vec<(String, String)> = Vec::new();
    for line in lines {
        let turn = found
            .turns
            .iter()
            .find(|t| t.line == *line)
            .ok_or_else(|| format!("line {line} is not a user turn of {session}"))?;
        let text = match quote.map(str::trim) {
            Some(q) if !turn.text.contains(q) => {
                return Err(format!("quote does not occur word for word in line {line}"))
            }
            Some(q) => q.to_string(),
            None => turn.text.clone(),
        };
        picked.push((text, format!("{}:{}#L{}", found.tool, session, line)));
    }
    picked
        .iter()
        .map(|(text, provenance)| add_constraint(root, project, provenance, text, None, None))
        .collect()
}

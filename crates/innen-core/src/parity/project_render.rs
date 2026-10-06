//! Project page render: every graph member of a project, grouped by kind.
//!
//! The page is the handoff document an agent reads before touching a project's
//! domain material. On 2026-09-02 it was consumed that way: the PCNe monthly
//! report spec (`03-output/research/2026-09-02-nhri-pcne-progress-report-spec.json`)
//! lists `04-index/projects/_generated/nhri-pcne.md` as `sources[0]`, ahead of
//! the research minutes, with `forbidden_cross_project_sources` naming
//! `project:itri-icope`. So the page has to carry who the project is, what
//! belongs to it, and when it was rendered — a stale or ambiguous page is worse
//! than none.
//!
//! ## Why membership is keyed on the id prefix, not the node type
//!
//! Node `type` is unreliable in practice. Across the `innen-wiki` graph every
//! `workstream:` node is typed `Task`, every `workspace:` node is typed
//! `Project`, and `meeting:` nodes are typed `Conversation`. The legacy
//! `_generated/` pages grouped by id prefix, which is why they listed
//! Workstreams at all. Prefix therefore wins; `type` is only a fallback for
//! nodes whose prefix carries no meaning (`t:1`, `e:1`, …).
//!
//! ## Why membership is bidirectional
//!
//! A project's members are not all `BELONGS_TO` inbound. `HAS_WORKSTREAM` runs
//! project → workstream, while `IMPLEMENTS`, `SUBJECT_OF` and `INFORMS` run
//! other-node → project. A previous version only matched `edge.to == project`,
//! so Workstreams, Codebases and Products could never appear.
//!
//! Retracted edges are skipped; validity windows are ignored
//! (`materialize(..., include_expired = true)`, same as `status`/`timeline`).
//! Unknown id (no node with that id, including unreadable journal) errors.

use std::path::Path;

use crate::graph::materialize;
use crate::journal::Journal;

/// A page section. A member matches on its id prefix first, then on its type.
struct Section {
    header: &'static str,
    prefixes: &'static [&'static str],
    types: &'static [&'static str],
}

/// Section order and naming follow the legacy `_generated/` pages, so a
/// regenerated page diffs cleanly against the one it replaces. Observed order
/// across the twelve surviving pages, e.g. `inhaler-coach.md`: Related projects,
/// Workstreams, Decisions, Tasks / blockers, Experiments, Datasets, Data
/// sessions, Models, Knowledge extraction gaps, Artifacts, Codebases /
/// workspaces, Sources, Organizations. Meetings, Conversations and Products slot
/// in beside the prefix they belong to.
const SECTIONS: &[Section] = &[
    Section {
        header: "## Related projects",
        prefixes: &["project"],
        types: &[],
    },
    Section {
        header: "## Workstreams",
        prefixes: &["workstream"],
        types: &["workstream"],
    },
    Section {
        header: "## Meetings",
        prefixes: &["meeting"],
        types: &[],
    },
    Section {
        header: "## Conversations",
        prefixes: &["conversation"],
        types: &["conversation"],
    },
    Section {
        header: "## Decisions",
        prefixes: &["decision"],
        types: &["decision"],
    },
    Section {
        header: "## Tasks / blockers",
        prefixes: &["task"],
        types: &["task"],
    },
    Section {
        header: "## Experiments",
        prefixes: &["experiment"],
        types: &["experiment"],
    },
    Section {
        header: "## Datasets",
        prefixes: &["dataset"],
        types: &["dataset"],
    },
    Section {
        header: "## Data sessions",
        prefixes: &["data-session"],
        types: &["data-session"],
    },
    Section {
        header: "## Models",
        prefixes: &["model"],
        types: &["model"],
    },
    Section {
        header: "## Knowledge extraction gaps",
        prefixes: &["harvest-gap"],
        types: &["harvest-gap"],
    },
    Section {
        header: "## Artifacts",
        prefixes: &["artifact", "planned-artifact"],
        types: &["artifact"],
    },
    Section {
        header: "## Codebases / workspaces",
        prefixes: &["workspace"],
        types: &["workspace"],
    },
    Section {
        header: "## Products / capabilities",
        prefixes: &["product"],
        types: &["product"],
    },
    Section {
        header: "## Sources",
        prefixes: &["source"],
        types: &["source"],
    },
    Section {
        header: "## Organizations",
        prefixes: &["organization"],
        types: &["organization"],
    },
];
fn prefix_of(id: &str) -> &str {
    id.split_once(':').map(|(p, _)| p).unwrap_or("")
}

/// Project entries from `04-index/projects/registry.json`, keyed by project id.
/// Returns an empty map when the file is absent or unparseable — the page still
/// renders, just without registry-sourced identity.
fn read_registry(root: &Path) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("04-index/projects/registry.json")) else {
        return out;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    let Some(projects) = value.get("projects").and_then(|v| v.as_array()) else {
        return out;
    };
    for entry in projects {
        if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
            out.insert(id.to_string(), entry.clone());
        }
    }
    out
}

fn section_for(id: &str, node_type: &str) -> Option<usize> {
    let prefix = prefix_of(id);
    let lowered = node_type.to_ascii_lowercase();
    SECTIONS
        .iter()
        .position(|s| s.prefixes.contains(&prefix))
        .or_else(|| {
            SECTIONS
                .iter()
                .position(|s| s.types.iter().any(|t| *t == lowered))
        })
}

/// One rendered bullet: label, lifecycle status, the edge that ties it to the
/// project, and whichever of date/summary/path the node carries.
struct Member {
    label: String,
    date: String,
    status: String,
    summary: String,
    location: String,
    edge: String,
}

fn field<'a>(node: &'a serde_json::Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

impl Member {
    fn render(&self) -> String {
        let mut line = String::from("- ");
        if !self.date.is_empty() {
            line.push_str(&self.date);
            line.push('｜');
        }
        line.push_str(&self.label);
        if !self.status.is_empty() {
            line.push_str(&format!(" [{}]", self.status));
        }
        if !self.edge.is_empty() {
            line.push_str(&format!(" `{}`", self.edge));
        }
        if !self.location.is_empty() {
            line.push_str(&format!(" — `{}`", self.location));
        }
        if !self.summary.is_empty() {
            line.push_str(&format!("：{}", self.summary));
        }
        line
    }

    /// Undated members first, then by date ascending, then by label — the order
    /// the legacy `_generated/` pages used, so a regenerated page diffs cleanly
    /// against the one it replaces. Returns owned fields because the key is
    /// rebuilt on every comparison.
    fn sort_key(&self) -> (bool, String, String) {
        (!self.date.is_empty(), self.date.clone(), self.label.clone())
    }
}

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
    let project_label = field(project, "label");
    let project_label = if project_label.is_empty() {
        target_id
    } else {
        project_label
    };
    // `organization` and `status` have a single writable home in
    // `04-index/projects/registry.json`; graph nodes only carry them for the
    // projects that happened to be seeded from the registry. Fall back to the
    // registry so a thin node does not render an unqualified title.
    let registry = read_registry(root);
    let slug = target_id.strip_prefix("project:").unwrap_or(target_id);
    let from_graph = |key: &str| -> String {
        let value = field(project, key);
        if value.is_empty() {
            registry
                .get(slug)
                .and_then(|e| e.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            value.to_string()
        }
    };
    let organization = from_graph("organization");

    let mut buckets: Vec<Vec<Member>> = SECTIONS.iter().map(|_| Vec::new()).collect();
    for edge in &materialized.edges {
        if edge.retracted {
            continue;
        }
        // Membership is bidirectional: the member is whichever endpoint is not
        // the project itself.
        let member_id = if edge.to == target_id {
            edge.from.as_str()
        } else if edge.from == target_id {
            edge.to.as_str()
        } else {
            continue;
        };
        if member_id == target_id {
            continue;
        }
        let Some(node) = materialized.nodes.get(member_id) else {
            continue;
        };
        let node_type = field(node, "type");
        let Some(index) = section_for(member_id, node_type) else {
            continue;
        };
        let label = field(node, "label");
        let label = if label.is_empty() { member_id } else { label };
        buckets[index].push(Member {
            label: label.to_string(),
            date: field(node, "date").to_string(),
            status: field(node, "status").to_string(),
            summary: field(node, "summary").to_string(),
            location: field(node, "path").lines().next().unwrap_or("").to_string(),
            edge: edge.edge.to_string(),
        });
    }

    let mut out = if organization.is_empty() {
        format!("# {project_label}\n\n")
    } else {
        format!("# {organization}｜{project_label}\n\n")
    };
    // The bare slug is what a reader passes back to `innen project`.
    out.push_str(&format!("- Project ID: `{slug}`\n"));
    let status = from_graph("status");
    if !status.is_empty() {
        out.push_str(&format!("- Status: `{status}`\n"));
    }
    out.push_str(&format!(
        "- Graph generated: `{}`\n\n",
        crate::journal::observed_utc_now()
    ));

    for (index, section) in SECTIONS.iter().enumerate() {
        let members = &mut buckets[index];
        members.sort_by_key(Member::sort_key);
        out.push_str(section.header);
        out.push_str("\n\n");
        for member in members.iter() {
            out.push_str(&member.render());
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(out)
}

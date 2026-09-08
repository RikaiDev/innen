use super::types::Cli;
#[derive(clap::Args)]
pub(super) struct SearchArgs {
    /// Keyword (tantivy query over label/body; empty matches nothing).
    #[arg(long)]
    pub(super) keyword: String,
    /// Max hits (default 20).
    #[arg(long, default_value_t = 20)]
    pub(super) limit: u16,
}
#[derive(clap::Args)]
pub(super) struct TimelineArgs {
    /// Optional prefix filter, e.g. `2026-09`.
    pub(super) filter: Option<String>,
}
#[derive(clap::Args)]
pub(super) struct ProjectArgs {
    /// Project id or slug. Omit for the portfolio of recorded tasks.
    pub(super) id: Option<String>,
    /// Brief pending tasks (default), evidence with history, or legacy full project page.
    #[arg(long, default_value = "brief", value_parser = ["brief", "evidence", "full"])]
    pub(super) view: String,
    /// Maximum task rows, with an explicit next_offset when more remain.
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=1000))]
    pub(super) limit: u16,
    /// Zero-based task offset for the next page.
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(serde::Serialize)]
pub(super) struct GuideJson {
    pub(super) text: String,
}

#[derive(serde::Serialize)]
pub(super) struct SearchHitJson {
    pub(super) node_id: String,
    pub(super) excerpt: String,
}

#[derive(serde::Serialize)]
pub(super) struct SearchJson {
    pub(super) hits: Vec<SearchHitJson>,
}

#[derive(serde::Serialize)]
pub(super) struct StatusJson {
    pub(super) nodes: u64,
    pub(super) edges: u64,
    pub(super) events: u64,
    pub(super) artifacts: u64,
}

#[derive(serde::Serialize)]
pub(super) struct TimelineEntryJson {
    pub(super) observed_utc: String,
    pub(super) op: String,
    pub(super) summary: String,
}

#[derive(serde::Serialize)]
pub(super) struct TimelineJson {
    pub(super) entries: Vec<TimelineEntryJson>,
}

#[derive(serde::Serialize)]
pub(super) struct ProjectJson {
    pub(super) id: String,
    pub(super) render: String,
}

#[derive(serde::Serialize)]
pub(super) struct ProfileJson {
    pub(super) render: String,
}

use super::util::{escape_tsv_field, is_human};
use clap::CommandFactory as _;

pub(super) fn cmd_completions(shell: &str) -> i32 {
    let clap_shell = match shell {
        "bash" => clap_complete::Shell::Bash,
        "zsh" => clap_complete::Shell::Zsh,
        "fish" => clap_complete::Shell::Fish,
        "powershell" => clap_complete::Shell::PowerShell,
        _ => {
            eprintln!("error: unknown shell: {shell}");
            return 1;
        }
    };
    let mut cmd = Cli::command();
    clap_complete::generate(clap_shell, &mut cmd, "innen", &mut std::io::stdout());
    0
}

pub(super) fn cmd_search(root: &std::path::Path, format: &str, args: &SearchArgs) -> i32 {
    let hits = innen_core::parity::search(root, &args.keyword, args.limit);
    if is_human(format) {
        println!("node_id\texcerpt");
        for h in &hits {
            println!(
                "{}\t{}",
                escape_tsv_field(&h.node_id),
                escape_tsv_field(&h.excerpt)
            );
        }
    } else {
        let out = SearchJson {
            hits: hits
                .into_iter()
                .map(|h| SearchHitJson {
                    node_id: h.node_id,
                    excerpt: h.excerpt,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("search output serializes")
        );
    }
    0
}

pub(super) fn cmd_project(root: &std::path::Path, format: &str, id: &str) -> i32 {
    match innen_core::parity::project_render(root, id) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProjectJson {
                        id: id.to_string(),
                        render,
                    })
                    .expect("project output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub(super) fn cmd_status(root: &std::path::Path, format: &str) -> i32 {
    let s = innen_core::parity::status(root);
    if is_human(format) {
        println!("nodes\tedges\tevents\tartifacts");
        println!("{}\t{}\t{}\t{}", s.nodes, s.edges, s.events, s.artifacts);
    } else {
        let out = StatusJson {
            nodes: s.nodes,
            edges: s.edges,
            events: s.events,
            artifacts: s.artifacts,
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("status output serializes")
        );
    }
    0
}

pub(super) fn cmd_profile(root: &std::path::Path, format: &str) -> i32 {
    match innen_core::parity::profile_render(root) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProfileJson { render })
                        .expect("profile output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub(super) fn cmd_timeline(root: &std::path::Path, format: &str, args: &TimelineArgs) -> i32 {
    let rows = innen_core::parity::timeline(root, args.filter.as_deref());
    if is_human(format) {
        println!("observed_utc\top\tsummary");
        for e in &rows {
            println!(
                "{}\t{}\t{}",
                escape_tsv_field(&e.observed_utc),
                escape_tsv_field(&e.op),
                escape_tsv_field(&e.summary)
            );
        }
    } else {
        let out = TimelineJson {
            entries: rows
                .into_iter()
                .map(|e| TimelineEntryJson {
                    observed_utc: e.observed_utc,
                    op: e.op,
                    summary: e.summary,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("timeline output serializes")
        );
    }
    0
}

pub(super) fn cmd_guide(format: &str) -> i32 {
    let text = innen_core::parity::guide_text();
    if is_human(format) {
        println!("{text}");
    } else {
        println!(
            "{}",
            serde_json::to_string(&GuideJson {
                text: text.to_string(),
            })
            .expect("guide output serializes")
        );
    }
    0
}

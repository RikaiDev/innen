mod artifact;
mod checkpoint;
mod commands;
mod config;
mod conversation;
mod delivery;
mod graph;
mod harvest;
mod hook;
mod index;
mod ledger;
mod local_discovery;
mod middleware;
mod navigation;
mod pickup;
mod query;
mod resume;
mod retention;
mod trace;
mod types;
mod unfinished;
mod util;

use clap::Parser;
use commands::{Commands, WikiOp};
use types::Cli;
use util::is_human;

fn resolve_root(
    cli_root: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, innen_core::config::RootResolutionError> {
    innen_core::config::resolve_root(cli_root)
}

pub(crate) fn run() -> i32 {
    let cli = Cli::parse();
    macro_rules! resolve_or_exit {
        ($cli_root:expr) => {
            match resolve_root($cli_root) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            }
        };
    }
    match &cli.command {
        Commands::Conversation(args) => conversation::cmd_conversation(&cli.format, args),
        Commands::Resume(args) => resume::cmd_resume(&cli.format, args),
        Commands::Checkpoint(args) => checkpoint::cmd_checkpoint(&cli.format, args),
        Commands::Unfinished(args) => unfinished::cmd_unfinished(&cli.format, args),
        Commands::Pickup(args) => pickup::cmd_pickup(&cli.format, args),
        Commands::Query(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            query::cmd_query(&root, &cli.format, args)
        }
        Commands::Graph { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                graph::GraphOp::Node(a) => graph::cmd_graph_node(&root, &cli.format, a),
                graph::GraphOp::Relate(a) => graph::cmd_graph_relate(&root, &cli.format, a),
                graph::GraphOp::Retract(a) => graph::cmd_graph_retract(&root, &cli.format, a),
            }
        }
        Commands::Index { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                index::IndexOp::Rebuild => index::cmd_index_rebuild(&root, &cli.format),
            }
        }
        Commands::Doctor => {
            let root = resolve_or_exit!(cli.root.clone());
            index::cmd_doctor(&root, &cli.format)
        }
        Commands::Config { op } => match op {
            config::ConfigOp::Get { key, global } => {
                if *global {
                    config::cmd_config_get_global(&cli.format, key)
                } else {
                    let root = resolve_or_exit!(cli.root.clone());
                    config::cmd_config_get(&root, &cli.format, key)
                }
            }
            config::ConfigOp::Set { key, value, global } => {
                if *global {
                    config::cmd_config_set_global(&cli.format, key, value)
                } else {
                    let root = resolve_or_exit!(cli.root.clone());
                    config::cmd_config_set(&root, &cli.format, key, value)
                }
            }
        },
        Commands::Completions { shell } => navigation::cmd_completions(shell),
        Commands::Guide => {
            // The guide is the operational source of truth, so it names the
            // root it resolved. An unresolvable root is reported, not fatal:
            // the navigation text is still worth reading.
            match resolve_root(cli.root.clone()) {
                Ok(root) => navigation::cmd_guide(&cli.format, Some(&root), None),
                Err(e) => navigation::cmd_guide(&cli.format, None, Some(&e.to_string())),
            }
        }
        Commands::Status => {
            let root = resolve_or_exit!(cli.root.clone());
            navigation::cmd_status(&root, &cli.format)
        }
        Commands::Timeline(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            navigation::cmd_timeline(&root, &cli.format, args)
        }
        Commands::Project(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            if args.view == "full" {
                match &args.id {
                    Some(id) => navigation::cmd_project(&root, &cli.format, id),
                    None => {
                        eprintln!("--view full requires a project id");
                        1
                    }
                }
            } else {
                let options = innen_core::project_brief::TaskOptions {
                    project: args.id.as_deref(),
                    all: args.view == "evidence",
                    detail: args.view == "evidence",
                    offset: args.offset,
                    limit: usize::from(args.limit),
                };
                match innen_core::project_brief::list(&root, &options) {
                    Ok(out) => {
                        if is_human(&cli.format) {
                            print!("{}", innen_core::project_brief::render(&out));
                        } else {
                            println!("{}", out);
                        }
                        0
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                }
            }
        }
        Commands::Profile => {
            let root = resolve_or_exit!(cli.root.clone());
            navigation::cmd_profile(&root, &cli.format)
        }
        Commands::Artifact { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                artifact::ArtifactOp::Add(a) => artifact::cmd_artifact_add(&root, &cli.format, a),
                artifact::ArtifactOp::AddTree(a) => {
                    artifact::cmd_artifact_add_tree(&root, &cli.format, a)
                }
                artifact::ArtifactOp::Finalize(a) => {
                    artifact::cmd_artifact_finalize(&root, &cli.format, a)
                }
                artifact::ArtifactOp::Ledger { op } => {
                    ledger::cmd_artifact_ledger(&root, &cli.format, op)
                }
            }
        }
        Commands::Harvest(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            harvest::cmd_harvest(&root, &cli.format, args)
        }
        Commands::Hook(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            hook::cmd_hook(&root, &cli.format, args)
        }
        Commands::Ingest => {
            let root = resolve_or_exit!(cli.root.clone());
            harvest::cmd_ingest(&root, &cli.format)
        }
        Commands::Retention(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            retention::cmd_retention(&root, &cli.format, args)
        }
        Commands::Middleware(args) => middleware::cmd_middleware(&cli.format, args),
        Commands::Trace(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            trace::cmd_trace(&root, args)
        }
        Commands::Wiki { op: WikiOp::Sync } => {
            let root = resolve_or_exit!(cli.root.clone());
            match innen_core::wiki_graph::sync(&root) {
                Ok(report) => {
                    println!(
                        "{}",
                        serde_json::to_string(&report).expect("wiki sync report serializes")
                    );
                    // A partial projection is still written, but it must not
                    // report success: something the operator expects in the
                    // graph is not there, and only the exit code says so.
                    if report.pages_skipped > 0 {
                        eprintln!(
                            "warning: {} page(s) skipped and left unchanged; see warnings",
                            report.pages_skipped
                        );
                        1
                    } else {
                        0
                    }
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    2
                }
            }
        }
    }
}

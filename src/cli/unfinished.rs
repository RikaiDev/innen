use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct UnfinishedArgs {
    /// Discover unfinished candidates across all projects (portfolio discovery).
    #[arg(long, conflicts_with = "project")]
    all_projects: bool,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to filter (auto, agy, codex, ...).
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Time cutoff (default yesterday+today; YYYY-MM-DD, RFC3339, 2d, 48h, today, yesterday).
    #[arg(long)]
    since: Option<String>,
}

pub(super) fn cmd_unfinished(format: &str, args: &UnfinishedArgs) -> i32 {
    let source_filter = if args.source == "auto" {
        None
    } else {
        match innen_core::conversation::Source::parse(&args.source) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };

    if args.all_projects {
        match innen_core::conversation::unfinished::find_all_unfinished_candidates(
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        ) {
            Ok(candidates) => {
                if is_human(format) {
                    if candidates.is_empty() {
                        println!("No unfinished candidate sessions found across all projects.");
                    } else {
                        println!("Unfinished candidate sessions across all projects:");
                        for c in &candidates {
                            println!(
                                "  - {} ({}, project: {}, modified: {}, confidence: {:?})",
                                c.id,
                                c.source.as_str(),
                                c.project,
                                c.modified.as_deref().unwrap_or("unknown"),
                                c.confidence
                            );
                            for r in &c.reasons {
                                println!("      * {}: {}", r.code, r.detail);
                            }
                            if let Some(ref cp) = c.checkpoint {
                                println!(
                                    "      Objective: {} [{}]",
                                    cp.objective,
                                    cp.status.as_str()
                                );
                            }
                            println!("      Resume with: innen resume {}", c.id);
                        }
                    }
                } else {
                    let out = serde_json::json!({
                        "status": "ok",
                        "all_projects": true,
                        "since": args.since,
                        "count": candidates.len(),
                        "candidates": candidates
                    });
                    println!("{}", serde_json::to_string(&out).unwrap());
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    } else {
        let project_dir = match &args.project {
            Some(p) => p.clone(),
            None => match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: failed to determine current working directory: {e}");
                    return 1;
                }
            },
        };

        match innen_core::conversation::unfinished::find_unfinished_candidates(
            &project_dir,
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        ) {
            Ok(candidates) => {
                if is_human(format) {
                    if candidates.is_empty() {
                        println!(
                            "No unfinished candidate sessions found for project {}.",
                            project_dir.display()
                        );
                    } else {
                        println!(
                            "Unfinished candidate sessions for project {}:",
                            project_dir.display()
                        );
                        for c in &candidates {
                            println!(
                                "  - {} ({}, modified: {}, confidence: {:?})",
                                c.id,
                                c.source.as_str(),
                                c.modified.as_deref().unwrap_or("unknown"),
                                c.confidence
                            );
                            for r in &c.reasons {
                                println!("      * {}: {}", r.code, r.detail);
                            }
                            if let Some(ref cp) = c.checkpoint {
                                println!(
                                    "      Objective: {} [{}]",
                                    cp.objective,
                                    cp.status.as_str()
                                );
                            }
                            println!("      Resume with: innen resume {}", c.id);
                        }
                    }
                } else {
                    let out = serde_json::json!({
                        "status": "ok",
                        "project": project_dir,
                        "since": args.since,
                        "count": candidates.len(),
                        "candidates": candidates
                    });
                    println!("{}", serde_json::to_string(&out).unwrap());
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    }
}

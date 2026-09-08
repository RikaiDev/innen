use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct PickupArgs {
    /// Discover candidates across all projects (portfolio discovery).
    #[arg(long, conflicts_with = "project")]
    all_projects: bool,
    /// Optional UUID or native session ID. If omitted, discovers recent unfinished candidate.
    uuid: Option<String>,
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

pub(super) fn cmd_pickup(format: &str, args: &PickupArgs) -> i32 {
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

    let target_result = if args.all_projects {
        innen_core::conversation::pickup::resolve_all_projects_pickup(
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        )
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
        innen_core::conversation::pickup::resolve_pickup(
            &project_dir,
            args.uuid.as_deref(),
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        )
    };

    match target_result {
        Ok(innen_core::conversation::pickup::PickupTarget::PickedUp(session)) => {
            if is_human(format) {
                println!(
                    "Picked up session: {} ({})",
                    session.session_id,
                    session.source.as_str()
                );
                println!("Project:        {}", session.project.display());
                if let Some(ref cp) = session.checkpoint {
                    println!("Status:         {}", cp.status.as_str());
                    println!("Objective:      {}", cp.objective);
                }
                println!("Confidence:     {:?}", session.evidence_pointers.confidence);
                println!("Reasons:");
                for r in &session.evidence_pointers.reasons {
                    println!("  - {}: {}", r.code, r.detail);
                }
                println!("Next action:    {}", session.next_action);
                println!("Verification:   {}", session.verification);
                println!("Resume command: {}", session.resume_command);
            } else {
                let out = serde_json::json!({
                    "status": "picked_up",
                    "session_id": session.session_id,
                    "source": session.source,
                    "project": session.project,
                    "checkpoint": session.checkpoint,
                    "evidence_pointers": session.evidence_pointers,
                    "next_action": session.next_action,
                    "verification": session.verification,
                    "resume_command": session.resume_command
                });
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            0
        }
        Ok(innen_core::conversation::pickup::PickupTarget::Ambiguous {
            project,
            count,
            candidates,
            message,
        }) => {
            let out = serde_json::json!({
                "status": "ambiguous",
                "project": project,
                "count": count,
                "candidates": candidates,
                "message": message
            });
            if is_human(format) {
                if project.to_string_lossy() == "all-projects" {
                    eprintln!("Multiple candidate sessions found across projects:");
                    for c in &candidates {
                        eprintln!(
                            "  - {} ({}, project: {}, modified: {}, confidence: {:?})",
                            c.id,
                            c.source.as_str(),
                            c.project,
                            c.modified.as_deref().unwrap_or("unknown"),
                            c.confidence
                        );
                    }
                } else {
                    eprintln!(
                        "Multiple candidate sessions found for project {}:",
                        project.display()
                    );
                    for c in &candidates {
                        eprintln!(
                            "  - {} ({}, modified: {}, confidence: {:?})",
                            c.id,
                            c.source.as_str(),
                            c.modified.as_deref().unwrap_or("unknown"),
                            c.confidence
                        );
                    }
                }
                eprintln!("Pick up explicit session with: innen pickup <session-id>");
            } else {
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            1
        }
        Ok(innen_core::conversation::pickup::PickupTarget::NotFound { project, message }) => {
            let out = serde_json::json!({
                "status": "not_found",
                "project": project,
                "count": 0,
                "candidates": [],
                "message": message
            });
            if is_human(format) {
                if project.to_string_lossy() == "all-projects" {
                    eprintln!("No candidate sessions found across any projects.");
                } else {
                    eprintln!(
                        "No candidate sessions found for project {}.",
                        project.display()
                    );
                }
                eprintln!("{message}");
            } else {
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

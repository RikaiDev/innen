use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct CheckpointArgs {
    #[command(subcommand)]
    op: Option<CheckpointOp>,
    /// Session UUID or native ID (when no subcommand provided).
    #[arg(long)]
    session: Option<String>,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(clap::Subcommand)]
pub(super) enum CheckpointOp {
    /// Record or update a progress checkpoint (append-only history).
    Record(Box<CheckpointRecordArgs>),
    /// Show the latest checkpoint for a session.
    Show(CheckpointShowArgs),
    /// List latest checkpoints for the project.
    List(CheckpointListArgs),
}

#[derive(clap::Args)]
pub(super) struct CheckpointRecordArgs {
    /// Session UUID or native ID. If omitted, checks project metadata for an unambiguous candidate.
    #[arg(long)]
    session: Option<String>,
    /// Tool source (auto, agy, codex, ...).
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Checkpoint status: active, blocked, completed, superseded.
    #[arg(long, default_value = "active", value_parser = ["active", "blocked", "completed", "superseded"])]
    status: String,
    /// High-level goal or objective.
    #[arg(long)]
    objective: Option<String>,
    /// Completed work items (comma-separated).
    #[arg(long, value_delimiter = ',')]
    completed: Vec<String>,
    /// Current evidence / observations (comma-separated).
    #[arg(long, value_delimiter = ',')]
    evidence: Vec<String>,
    /// Known blockers or open questions (comma-separated).
    #[arg(long, value_delimiter = ',')]
    blocker: Vec<String>,
    /// Immediate next action for continuation.
    #[arg(long)]
    next_action: Option<String>,
    /// Verification command or criteria.
    #[arg(long)]
    verification: Option<String>,
    /// Load checkpoint JSON from file or '-' for stdin.
    #[arg(long)]
    file: Option<String>,
}

#[derive(clap::Args)]
pub(super) struct CheckpointShowArgs {
    /// Session UUID or native ID. If omitted, checks project metadata for an unambiguous candidate.
    session: Option<String>,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(clap::Args)]
pub(super) struct CheckpointListArgs {
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

pub(super) fn cmd_checkpoint(format: &str, args: &CheckpointArgs) -> i32 {
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

    match &args.op {
        Some(CheckpointOp::Record(rec)) => {
            let checkpoint = if let Some(ref file_arg) = rec.file {
                let content = if file_arg == "-" {
                    let mut s = String::new();
                    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s) {
                        eprintln!("error: failed to read checkpoint JSON from stdin: {e}");
                        return 1;
                    }
                    s
                } else {
                    match std::fs::read_to_string(file_arg) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("error: failed to read checkpoint file {file_arg}: {e}");
                            return 1;
                        }
                    }
                };
                match serde_json::from_str::<innen_core::conversation::Checkpoint>(&content) {
                    Ok(cp) => cp,
                    Err(e) => {
                        eprintln!("error: invalid checkpoint JSON: {e}");
                        return 1;
                    }
                }
            } else {
                let session = match &rec.session {
                    Some(s) => s.clone(),
                    None => {
                        match innen_core::conversation::resume::resolve_resume_target(
                            &project_dir,
                            &rec.source,
                            None,
                        ) {
                            Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                                c.id
                            }
                            Ok(innen_core::conversation::resume::ResumeTarget::Ambiguous {
                                ..
                            }) => {
                                eprintln!("error: multiple candidate sessions found; specify --session <id>");
                                return 1;
                            }
                            _ => {
                                eprintln!("error: --session <id> is required when no unambiguous session exists");
                                return 1;
                            }
                        }
                    }
                };

                let existing = innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                    &project_dir,
                    &session,
                )
                .ok()
                .flatten();

                let status = match innen_core::conversation::CheckpointStatus::parse(&rec.status) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 1;
                    }
                };

                let objective = if let Some(ref obj) = rec.objective {
                    obj.clone()
                } else if let Some(ref ex) = existing {
                    ex.objective.clone()
                } else {
                    eprintln!("error: --objective is required for a new checkpoint");
                    return 1;
                };

                let source = if rec.source != "auto" {
                    match innen_core::conversation::Source::parse(&rec.source) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 1;
                        }
                    }
                } else if let Some(ref ex) = existing {
                    ex.source
                } else {
                    innen_core::conversation::Source::Codex
                };

                let completed_work = if !rec.completed.is_empty() {
                    rec.completed.clone()
                } else if let Some(ref ex) = existing {
                    ex.completed_work.clone()
                } else {
                    Vec::new()
                };

                let current_evidence = if !rec.evidence.is_empty() {
                    rec.evidence.clone()
                } else if let Some(ref ex) = existing {
                    ex.current_evidence.clone()
                } else {
                    Vec::new()
                };

                let blockers = if !rec.blocker.is_empty() {
                    rec.blocker.clone()
                } else if let Some(ref ex) = existing {
                    ex.blockers.clone()
                } else {
                    Vec::new()
                };

                let next_action = rec.next_action.clone().unwrap_or_else(|| {
                    existing
                        .as_ref()
                        .map(|e| e.next_action.clone())
                        .unwrap_or_default()
                });

                let verification = rec.verification.clone().unwrap_or_else(|| {
                    existing
                        .as_ref()
                        .map(|e| e.verification.clone())
                        .unwrap_or_default()
                });

                innen_core::conversation::checkpoint::new_checkpoint(
                    session,
                    source,
                    project_dir.clone(),
                    status,
                    objective,
                    completed_work,
                    current_evidence,
                    blockers,
                    next_action,
                    verification,
                )
            };

            if let Err(e) =
                innen_core::conversation::checkpoint::record_checkpoint(&project_dir, &checkpoint)
            {
                eprintln!("error: {e}");
                return 1;
            }

            if is_human(format) {
                println!(
                    "Recorded checkpoint for session {} ({})",
                    checkpoint.session,
                    checkpoint.status.as_str()
                );
                println!("  Objective:   {}", checkpoint.objective);
                println!("  Status:      {}", checkpoint.status.as_str());
                println!("  Next action: {}", checkpoint.next_action);
            } else {
                let out = serde_json::json!({
                    "status": "recorded",
                    "checkpoint": checkpoint
                });
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            0
        }
        Some(CheckpointOp::Show(show_args)) => {
            let session = match &show_args.session {
                Some(s) => s.clone(),
                None => match &args.session {
                    Some(s) => s.clone(),
                    None => {
                        match innen_core::conversation::resume::resolve_resume_target(
                            &project_dir,
                            "auto",
                            None,
                        ) {
                            Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                                c.id
                            }
                            _ => {
                                eprintln!("error: specify session ID with: innen checkpoint show <session-id>");
                                return 1;
                            }
                        }
                    }
                },
            };

            match innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                &project_dir,
                &session,
            ) {
                Ok(Some(cp)) => {
                    if is_human(format) {
                        println!("Session:      {} ({})", cp.session, cp.source.as_str());
                        println!("Status:       {}", cp.status.as_str());
                        println!("Objective:    {}", cp.objective);
                        println!("Updated:      {}", cp.updated_at);
                        println!("Next action:  {}", cp.next_action);
                        println!("Verification: {}", cp.verification);
                        if !cp.completed_work.is_empty() {
                            println!("Completed work:");
                            for item in &cp.completed_work {
                                println!("  - {item}");
                            }
                        }
                        if !cp.current_evidence.is_empty() {
                            println!("Current evidence:");
                            for item in &cp.current_evidence {
                                println!("  - {item}");
                            }
                        }
                        if !cp.blockers.is_empty() {
                            println!("Blockers:");
                            for item in &cp.blockers {
                                println!("  - {item}");
                            }
                        }
                    } else {
                        println!("{}", serde_json::to_string(&cp).unwrap());
                    }
                    0
                }
                Ok(None) => {
                    let out = serde_json::json!({
                        "status": "not_found",
                        "session": session,
                        "message": format!("no checkpoint found for session {session}")
                    });
                    if is_human(format) {
                        eprintln!("No checkpoint found for session {session}.");
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
        Some(CheckpointOp::List(_)) | None => {
            if let Some(ref session) = args.session {
                return cmd_checkpoint(
                    format,
                    &CheckpointArgs {
                        op: Some(CheckpointOp::Show(CheckpointShowArgs {
                            session: Some(session.clone()),
                            project: Some(project_dir),
                        })),
                        session: None,
                        project: None,
                    },
                );
            }
            match innen_core::conversation::checkpoint::latest_checkpoints(&project_dir) {
                Ok(list) => {
                    if is_human(format) {
                        if list.is_empty() {
                            println!(
                                "No checkpoints found for project {}.",
                                project_dir.display()
                            );
                        } else {
                            println!("Checkpoints for project {}:", project_dir.display());
                            for cp in &list {
                                println!(
                                    "  - {} [{}] {} (updated: {})",
                                    cp.session,
                                    cp.status.as_str(),
                                    cp.objective,
                                    cp.updated_at
                                );
                            }
                        }
                    } else {
                        let out = serde_json::json!({
                            "status": "ok",
                            "project": project_dir,
                            "count": list.len(),
                            "checkpoints": list
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
}

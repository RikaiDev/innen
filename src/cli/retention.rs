use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct RetentionArgs {
    #[command(subcommand)]
    op: RetentionOp,
}

#[derive(clap::Subcommand)]
enum RetentionOp {
    /// Dry-run inventory with exact blocked reasons (default retention: 45 days).
    Plan(PlanArgs),
    /// Record a knowledge-extraction proof for the current native bytes.
    Attest(AttestArgs),
    /// Revalidate every gate and optionally delete one exact native session.
    Purge(PurgeArgs),
    /// Process every eligible session independently; one failure never stops the sweep.
    Sweep(SweepArgs),
}

#[derive(clap::Args)]
struct CommonSessionArgs {
    /// Native session UUID or OpenCode ses_ id.
    id: String,
    /// Coding tool owning the native store.
    #[arg(long, value_parser = ["agy", "antigravity", "codex", "claude", "opencode"])]
    source: String,
    /// Override the native store root.
    #[arg(long)]
    source_root: Option<PathBuf>,
}

#[derive(clap::Args)]
struct PlanArgs {
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "opencode"])]
    source: String,
    #[arg(long)]
    source_root: Option<PathBuf>,
    #[arg(long, default_value_t = 45)]
    retention_days: u64,
    /// Emit every session assessment. Default output is a bounded summary.
    #[arg(long)]
    details: bool,
}

#[derive(clap::Args)]
struct AttestArgs {
    #[command(flatten)]
    session: CommonSessionArgs,
    /// Durable knowledge node linked to the canonical conversation node.
    #[arg(long = "knowledge-node", required = true)]
    knowledge_nodes: Vec<String>,
    /// Explicitly attest that no live process owns this session.
    #[arg(long)]
    session_closed: bool,
}

#[derive(clap::Args)]
struct PurgeArgs {
    #[command(flatten)]
    session: CommonSessionArgs,
    #[arg(long, default_value_t = 45)]
    retention_days: u64,
    /// Perform deletion. Without this flag the command is a dry-run proof check.
    #[arg(long)]
    execute: bool,
}

#[derive(clap::Args)]
struct SweepArgs {
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "opencode"])]
    source: String,
    #[arg(long)]
    source_root: Option<PathBuf>,
    #[arg(long, default_value_t = 45)]
    retention_days: u64,
    /// Perform deletion. Without this flag the command reports eligible totals.
    #[arg(long)]
    execute: bool,
    /// Include per-session outcomes. Default output is a bounded summary.
    #[arg(long)]
    details: bool,
}

pub(super) fn cmd_retention(root: &std::path::Path, format: &str, args: &RetentionArgs) -> i32 {
    let result = match &args.op {
        RetentionOp::Plan(args) => {
            if args.source_root.is_some() && args.source == "auto" {
                Err("--source-root requires a concrete --source".to_string())
            } else {
                let source = if args.source == "auto" {
                    None
                } else {
                    match innen_core::conversation::Source::parse(&args.source) {
                        Ok(source) => Some(source),
                        Err(error) => return fail(&error.to_string()),
                    }
                };
                innen_core::session_retention::inventory(
                    root,
                    source,
                    args.source_root.as_deref(),
                    args.retention_days,
                )
                .and_then(|value| {
                    let output = if args.details {
                        serde_json::to_value(value)
                    } else {
                        serde_json::to_value(innen_core::session_retention::summarize_inventory(
                            &value,
                        ))
                    };
                    output.map_err(|error| {
                        innen_core::session_retention::Error::Proof(error.to_string())
                    })
                })
                .map_err(|error| error.to_string())
            }
        }
        RetentionOp::Attest(args) => innen_core::session_retention::attest(
            root,
            &args.session.source,
            &args.session.id,
            args.session.source_root.as_deref(),
            &args.knowledge_nodes,
            args.session_closed,
        )
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| innen_core::session_retention::Error::Proof(error.to_string()))
        })
        .map_err(|error| error.to_string()),
        RetentionOp::Purge(args) => innen_core::session_retention::purge(
            root,
            &args.session.source,
            &args.session.id,
            args.session.source_root.as_deref(),
            args.retention_days,
            args.execute,
        )
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| innen_core::session_retention::Error::Proof(error.to_string()))
        })
        .map_err(|error| error.to_string()),
        RetentionOp::Sweep(args) => {
            if args.source_root.is_some() && args.source == "auto" {
                Err("--source-root requires a concrete --source".to_string())
            } else {
                let source = if args.source == "auto" {
                    None
                } else {
                    match innen_core::conversation::Source::parse(&args.source) {
                        Ok(source) => Some(source),
                        Err(error) => return fail(&error.to_string()),
                    }
                };
                innen_core::session_retention::sweep(
                    root,
                    source,
                    args.source_root.as_deref(),
                    args.retention_days,
                    args.execute,
                )
                .and_then(|receipt| {
                    let value = if args.details {
                        serde_json::to_value(receipt)
                    } else {
                        serde_json::to_value(serde_json::json!({
                            "schema": receipt.schema,
                            "total": receipt.total,
                            "eligible": receipt.eligible,
                            "blocked": receipt.blocked,
                            "removed": receipt.removed,
                            "failed": receipt.failed,
                            "native_bytes_removed": receipt.native_bytes_removed,
                            "executed": receipt.executed,
                        }))
                    };
                    value.map_err(|error| {
                        innen_core::session_retention::Error::Proof(error.to_string())
                    })
                })
                .map_err(|error| error.to_string())
            }
        }
    };
    match result {
        Ok(value) => {
            if format == "human" {
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!("{}", serde_json::to_string(&value).unwrap());
            }
            0
        }
        Err(error) => fail(&error),
    }
}

fn fail(error: &str) -> i32 {
    eprintln!("error: {error}");
    1
}

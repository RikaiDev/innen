#[derive(clap::Args)]
pub(super) struct HarvestArgs {
    /// Dry-run flag (accepted for compat; harvest is always a dry-run check).
    #[arg(long, default_value_t = false)]
    pub(super) check: bool,
    /// Also inventory native coding-tool sessions and their extraction gates.
    #[arg(long)]
    pub(super) coding_sessions: bool,
    /// Emit every coding-session assessment instead of a bounded summary.
    #[arg(long, requires = "coding_sessions")]
    pub(super) coding_session_details: bool,
    /// Hot-session retention window used by --coding-sessions.
    #[arg(long, default_value_t = 45)]
    pub(super) retention_days: u64,
    /// Restrict native session inventory to one source.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "opencode"])]
    pub(super) source: String,
    /// Override the native store root; requires a concrete --source.
    #[arg(long, requires = "source")]
    pub(super) source_root: Option<std::path::PathBuf>,
}

use super::util::{escape_tsv_field, is_human};

pub(super) fn cmd_ingest(root: &std::path::Path, format: &str) -> i32 {
    // Thin call: appends + watermark advance live in
    // innen-core::harvest::ingest::run. Credential hits are reported, never
    // appended.
    let report = innen_core::harvest::ingest::run(root);
    if is_human(format) {
        println!("added\t{}", report.added);
        if !report.skipped.is_empty() {
            println!("path\tpattern\tpreview");
            for s in &report.skipped {
                println!(
                    "{}\t{}\t{}",
                    escape_tsv_field(&s.path),
                    escape_tsv_field(&s.pattern),
                    escape_tsv_field(&s.preview)
                );
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string(&report).expect("ingest report serializes")
        );
    }
    0
}

pub(super) fn cmd_harvest(root: &std::path::Path, format: &str, args: &HarvestArgs) -> i32 {
    // Thin call: dry-run report lives in innen-core::harvest (never appends,
    // never advances the watermark). Core structs serialize in field order,
    // which is the JSON wire order.
    let report = innen_core::harvest::check(root);
    if args.coding_sessions {
        if args.source_root.is_some() && args.source == "auto" {
            eprintln!("error: --source-root requires a concrete --source");
            return 1;
        }
        let source = if args.source == "auto" {
            None
        } else {
            match innen_core::conversation::Source::parse(&args.source) {
                Ok(source) => Some(source),
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            }
        };
        match innen_core::session_retention::inventory(
            root,
            source,
            args.source_root.as_deref(),
            args.retention_days,
        ) {
            Ok(sessions) => {
                if args.coding_session_details && is_human(format) {
                    println!("source\tsession\tmodified\teligible\tbytes\tblockers");
                    for session in sessions {
                        println!(
                            "{}\t{}\t{}\t{}\t{}\t{}",
                            escape_tsv_field(&session.source),
                            escape_tsv_field(&session.session_id),
                            escape_tsv_field(session.modified.as_deref().unwrap_or("unknown")),
                            session.eligible,
                            session.source_bytes,
                            escape_tsv_field(&session.blockers.join("; ")),
                        );
                    }
                } else if args.coding_session_details {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "inbox": report,
                            "coding_sessions": sessions,
                        }))
                        .expect("harvest session report serializes")
                    );
                } else {
                    let summary = innen_core::session_retention::summarize_inventory(&sessions);
                    if is_human(format) {
                        println!(
                            "coding sessions\t{}\neligible\t{}\neligible bytes\t{}",
                            summary.total, summary.eligible, summary.eligible_bytes
                        );
                        for (source, counts) in summary.by_source {
                            println!(
                                "source\t{}\t{}\t{}",
                                escape_tsv_field(&source),
                                counts.total,
                                counts.eligible
                            );
                        }
                        for (code, count) in summary.blocker_codes {
                            println!("blocker\t{}\t{}", escape_tsv_field(&code), count);
                        }
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "inbox": report,
                                "coding_sessions": summary,
                            }))
                            .expect("harvest session summary serializes")
                        );
                    }
                }
                return 0;
            }
            Err(error) => {
                eprintln!("error: {error}");
                return 1;
            }
        }
    }
    if is_human(format) {
        println!("id\tstatus\tpath");
        for tap in &report.taps {
            for f in &tap.new_files {
                println!(
                    "{}\tnew\t{}",
                    escape_tsv_field(&tap.id),
                    escape_tsv_field(f)
                );
            }
            for s in &tap.skipped {
                println!(
                    "{}\tskipped\t{}",
                    escape_tsv_field(&tap.id),
                    escape_tsv_field(s)
                );
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string(&report).expect("harvest report serializes")
        );
    }
    0
}

// P2 `harvest --check`: dry-run only (no appends, no watermark advance).

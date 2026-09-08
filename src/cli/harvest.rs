#[derive(clap::Args)]
pub(super) struct HarvestArgs {
    /// Dry-run flag (accepted for compat; harvest is always a dry-run check).
    #[arg(long, default_value_t = false)]
    pub(super) check: bool,
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

pub(super) fn cmd_harvest(root: &std::path::Path, format: &str, _args: &HarvestArgs) -> i32 {
    // Thin call: dry-run report lives in innen-core::harvest (never appends,
    // never advances the watermark). Core structs serialize in field order,
    // which is the JSON wire order.
    let report = innen_core::harvest::check(root);
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

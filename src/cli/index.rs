#[derive(clap::Subcommand)]
pub(super) enum IndexOp {
    /// Drop derived files and rebuild from the journal.
    Rebuild,
}

use super::util::{escape_tsv_field, is_human};

pub(super) fn cmd_lint(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::lint(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

pub(super) fn cmd_doctor(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::run(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

pub(super) fn print_report_json(report: &innen_core::doctor::Report) {
    println!(
        "{}",
        serde_json::to_string(report).expect("report serializes")
    );
}

pub(super) fn cmd_index_rebuild(root: &std::path::Path, format: &str) -> i32 {
    match innen_core::index::rebuild(root) {
        Ok(()) => {
            if is_human(format) {
                println!("index\trebuilt");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"ok": true}))
                        .expect("index output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub(super) fn print_report_human(report: &innen_core::doctor::Report) {
    // Bounded P1 human table: doctor checks (TSV-escaped, 200-char cap).
    println!("check\tok\tdetail");
    for c in &report.checks {
        println!(
            "{}\t{}\t{}",
            escape_tsv_field(&c.name),
            if c.ok { "ok" } else { "FAIL" },
            escape_tsv_field(&c.detail)
        );
    }
    println!("exit_code\t{}", report.exit_code);
}

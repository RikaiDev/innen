use super::super::util::is_human;
use super::args::LedgerOp;
use super::mirror::{
    existing_ledger_project_id, ledger_source_kind, ledger_stage, mirror_ledger_event,
    mirror_ledger_receipt,
};

pub(in crate::cli) fn cmd_artifact_ledger(
    kb_root: &std::path::Path,
    format: &str,
    op: &LedgerOp,
) -> i32 {
    use innen_core::artifact::ledger::{AddOptions, InitOptions, Ledger};
    match op {
        LedgerOp::Init(args) => match Ledger::init(InitOptions {
            root: args.archive_root.clone(),
            project_id: args.project_id.clone(),
            title: args.title.clone(),
            downloads_root: args.downloads_root.clone(),
            categories: args.categories.clone(),
        }) {
            Ok(_) => {
                if is_human(format) {
                    println!("ledger initialized\t{}", args.archive_root.display());
                } else {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "initialized",
                            "root": args.archive_root,
                        })
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        LedgerOp::Add(args) => {
            let source_kind = match ledger_source_kind(&args.source_kind) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let stage = match ledger_stage(&args.stage) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let ledger_project_id = match existing_ledger_project_id(&args.archive_root) {
                Ok(Some(id)) => id,
                Ok(None) => args
                    .archive_project_id
                    .clone()
                    .unwrap_or_else(|| args.owner_project.clone()),
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let ledger = match Ledger::init(InitOptions {
                root: args.archive_root.clone(),
                project_id: ledger_project_id,
                title: args
                    .archive_root
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("")
                    .to_string(),
                downloads_root: args.downloads_root.clone(),
                categories: vec![],
            }) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let receipt = match ledger.add(AddOptions {
                path: args.file.clone(),
                document_id: args.document_id.clone(),
                source_kind,
                role: args.role.clone(),
                stage,
                owner_project: args.owner_project.clone(),
                relation: args.relation.clone(),
                document_date: args.document_date.clone(),
                authority: args.authority.clone(),
                evidence_status: args.evidence_status.clone(),
                provenance: Some(args.provenance.clone()),
                approval_valid_from: args.approval_valid_from.clone(),
                approval_valid_until: args.approval_valid_until.clone(),
                downloads_root: args.downloads_root.clone(),
            }) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            if let Err(e) =
                mirror_ledger_receipt(kb_root, &receipt, args.archive_project_id.as_deref())
            {
                eprintln!("error: {e}");
                return 1;
            }
            if is_human(format) {
                println!("receipt\t{}\t{}", receipt.id, receipt.path);
            } else {
                println!(
                    "{}",
                    serde_json::json!({"receipt": receipt, "kb_sync": "ok"})
                );
            }
            0
        }
        LedgerOp::Event(args) => {
            use innen_core::artifact::ledger::{Ledger, LedgerEvent};
            let ledger = match Ledger::init(InitOptions {
                root: args.archive_root.clone(),
                project_id: args.project_id.clone(),
                title: args.title.clone(),
                downloads_root: None,
                categories: vec![],
            }) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let event = match args.kind.to_ascii_lowercase().as_str() {
                "note" => LedgerEvent::Note {
                    receipt_id: args.receipt_id.clone(),
                    note: args.note.clone().unwrap_or_default(),
                    event_date: args.event_date.clone(),
                },
                "supersede" => LedgerEvent::Supersede {
                    receipt_id: args.receipt_id.clone(),
                    superseded_receipt_id: match &args.superseded_receipt_id {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("error: --superseded-receipt-id is required for supersede");
                            return 1;
                        }
                    },
                    event_date: args.event_date.clone(),
                },
                "relocate" => LedgerEvent::Relocate {
                    receipt_id: args.receipt_id.clone(),
                    new_path: match &args.new_path {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("error: --new-path is required for relocate");
                            return 1;
                        }
                    },
                    expected_sha256: match &args.expected_sha256 {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("error: --expected-sha256 is required for relocate");
                            return 1;
                        }
                    },
                    event_date: args.event_date.clone(),
                },
                "reclassify" => LedgerEvent::Reclassify {
                    receipt_id: args.receipt_id.clone(),
                    new_owner_project: args.new_owner_project.clone(),
                    new_relation: match &args.new_relation {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("error: --new-relation is required for reclassify");
                            return 1;
                        }
                    },
                    provenance: match &args.provenance {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("error: --provenance is required for reclassify");
                            return 1;
                        }
                    },
                    event_date: args.event_date.clone(),
                },
                other => {
                    eprintln!("error: invalid ledger event kind: {other}");
                    return 1;
                }
            };
            let previous = match ledger.receipt(&args.receipt_id) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            match ledger.event(event) {
                Ok(record) => {
                    let current = match ledger.receipt(&args.receipt_id) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 1;
                        }
                    };
                    if let Err(e) = mirror_ledger_event(
                        kb_root,
                        &previous,
                        &current,
                        &record,
                        args.archive_project_id.as_deref(),
                    ) {
                        eprintln!("error: {e}");
                        return 1;
                    }
                    if is_human(format) {
                        println!("event\t{}\t{}", args.kind, args.receipt_id);
                    } else {
                        println!(
                            "{}",
                            serde_json::json!({
                                "event": args.kind,
                                "receipt_id": args.receipt_id,
                                "sequence": record.sequence,
                            })
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
        LedgerOp::Check(args) => match Ledger::init(InitOptions {
            root: args.archive_root.clone(),
            project_id: args.project_id.clone(),
            title: args.title.clone(),
            downloads_root: None,
            categories: vec![],
        })
        .and_then(|ledger| ledger.check())
        {
            Ok(report) => {
                if is_human(format) {
                    println!(
                        "active\t{}\nmissing\t{}\nchanged\t{}",
                        report.active,
                        report.missing.len(),
                        report.changed.len()
                    );
                } else {
                    println!(
                        "{}",
                        serde_json::json!({"active": report.active, "missing": report.missing, "changed": report.changed, "superseded": report.superseded})
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

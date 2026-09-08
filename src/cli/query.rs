#[derive(clap::Args)]
pub(super) struct QueryArgs {
    /// Query string (natural text or literal identifier/filename).
    #[arg(long)]
    pub(super) q: String,
    /// Evaluate pinned fact alternatives and dependency closure against the journal.
    #[arg(long, conflicts_with_all = ["as_of", "include_expired", "view", "limit", "offset"])]
    pub(super) evidence_contract: Option<PathBuf>,
    /// Prepare inspected source context for an external model; never invokes one.
    #[arg(
        long,
        requires = "evidence_contract",
        conflicts_with = "proposal_input"
    )]
    pub(super) prepare_proposal: bool,
    /// Validate an external model's cited task proposal against the trusted scope.
    #[arg(long, requires = "evidence_contract", group = "proposal_input")]
    pub(super) task_proposal: Option<PathBuf>,
    /// Validate ID-only output inside a controller-bound context envelope.
    #[arg(long, requires = "evidence_contract", group = "proposal_input")]
    pub(super) task_draft: Option<PathBuf>,
    /// Prepare source IDs and a native model output schema.
    #[arg(long, requires = "prepare_proposal")]
    pub(super) evidence_ids: bool,
    /// Caller review receipt bound to exact context and proposal hashes.
    #[arg(long, requires = "proposal_input")]
    pub(super) proposal_review: Option<PathBuf>,
    /// As-of cutoff `YYYY-MM-DDTHH:MM:SSZ` (defaults to now).
    #[arg(long = "as-of")]
    pub(super) as_of: Option<String>,
    /// Max hits (default 20, max 100; >100 clamps with `truncated`).
    #[arg(long, default_value_t = 20)]
    pub(super) limit: u16,
    /// Zero-based task/event offset for pagination.
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
    /// Bypass validity filtering (keep expired edges).
    #[arg(long = "include-expired")]
    pub(super) include_expired: bool,
    /// View mode: context (default compact task-context brief), hits (legacy FTS+graph hits), or evidence (full payload + physical history).
    #[arg(long, default_value = "context", value_parser = ["context", "hits", "evidence"])]
    pub(super) view: String,
}

use super::util::{escape_tsv_field, is_human};
use std::path::PathBuf;

pub(super) fn cmd_query(root: &std::path::Path, format: &str, args: &QueryArgs) -> i32 {
    if let Some(path) = &args.evidence_contract {
        let result = (|| -> Result<serde_json::Value, String> {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
                return Err("evidence contract exceeds 1 MiB bound".into());
            }
            let contract: innen_core::evidence_closure::Contract =
                serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if args.prepare_proposal {
                let raw = innen_core::evidence_closure::read_journal(root)?;
                let mut out = if args.evidence_ids {
                    innen_core::task_proposal::id_context(&raw, &contract, &args.q)?
                } else {
                    innen_core::evidence_closure::proposal_context(&raw, &contract, &args.q)?
                };
                out["resolution"] = serde_json::json!("proposal_context_prepared");
                Ok(out)
            } else if let Some(path) = args.task_proposal.as_ref().or(args.task_draft.as_ref()) {
                fn read_json(path: &std::path::Path) -> Result<serde_json::Value, String> {
                    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
                        return Err("proposal/review file exceeds 1 MiB".into());
                    }
                    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())
                }
                let raw = innen_core::evidence_closure::read_journal(root)?;
                let proposal = if args.task_draft.is_some() {
                    let draft = serde_json::from_value::<innen_core::task_proposal::DraftEnvelope>(
                        read_json(path)?,
                    )
                    .map_err(|e| e.to_string())?;
                    innen_core::task_proposal::bind_draft(&raw, &contract, &args.q, draft)?
                } else {
                    serde_json::from_value::<innen_core::task_proposal::Proposal>(read_json(path)?)
                        .map_err(|e| e.to_string())?
                };
                let review = args
                    .proposal_review
                    .as_ref()
                    .map(|p| {
                        serde_json::from_value::<innen_core::task_proposal::Review>(read_json(p)?)
                            .map_err(|e| e.to_string())
                    })
                    .transpose()?;
                innen_core::task_proposal::evaluate(
                    &raw,
                    &contract,
                    &args.q,
                    &proposal,
                    review.as_ref(),
                )
            } else {
                innen_core::evidence_closure::query(root, &contract, &args.q)
            }
        })();
        return match result {
            Ok(out) => {
                println!(
                    "{}",
                    if is_human(format) {
                        serde_json::to_string_pretty(&out).unwrap()
                    } else {
                        out.to_string()
                    }
                );
                if out["resolution"] == "covered"
                    || out["resolution"] == "proposal_context_prepared"
                {
                    0
                } else {
                    2
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    if args.view == "hits" {
        let params = innen_core::query::QueryParams {
            q: args.q.clone(),
            as_of: args.as_of.clone(),
            limit: args.limit,
            include_expired: args.include_expired,
        };
        match innen_core::query::query(root, &params) {
            Ok(out) => {
                if is_human(format) {
                    print_query_human(&out);
                } else {
                    print_query_json(&out);
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    } else {
        let options = innen_core::task_entry::TaskEntryOptions {
            q: args.q.clone(),
            as_of: args.as_of.clone(),
            limit: usize::from(args.limit),
            offset: args.offset,
            include_expired: args.include_expired,
            view: args.view.clone(),
        };
        match innen_core::task_entry::task_entry(root, &options) {
            Ok(out) => {
                if is_human(format) {
                    print!("{}", innen_core::task_entry::render(&out));
                } else {
                    println!(
                        "{}",
                        serde_json::to_string(&out).expect("task entry output serializes")
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
}

pub(super) fn print_query_human(out: &innen_core::query::QueryOutput) {
    // Bounded P1 human table: id/kind/score/why (+ label for readability).
    // All string fields are TSV-escaped (tabs/newlines encoded, 200-char cap).
    println!("id\tkind\tscore\twhy\tlabel");
    for h in &out.hits {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            escape_tsv_field(&h.node_id),
            escape_tsv_field(&h.kind),
            h.score,
            escape_tsv_field(&h.why),
            escape_tsv_field(&h.label)
        );
    }
    if !out.warnings.is_empty() {
        println!("warnings: {}", out.warnings.join(", "));
    }
}

pub(super) fn print_query_json(out: &innen_core::query::QueryOutput) {
    println!(
        "{}",
        serde_json::to_string(out).expect("query output serializes")
    );
}

#[derive(clap::Args)]
pub(super) struct TraceArgs {
    #[arg(long, required_unless_present = "expand", conflicts_with = "expand")]
    pub(super) q: Option<String>,
    /// Expand a hash-addressed trace source cache record against current bytes.
    #[arg(long, requires_all = ["record", "expect_sha256"])]
    pub(super) expand: Option<String>,
    #[arg(long, requires = "expand")]
    pub(super) record: Option<usize>,
    #[arg(long, requires = "expand")]
    pub(super) expect_sha256: Option<String>,
    #[arg(long, default_value_t = 5)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 8)]
    pub(super) max_sources: usize,
    #[arg(long, default_value_t = 33_554_432)]
    pub(super) max_bytes: u64,
    #[arg(long, default_value_t = 50_000)]
    pub(super) max_records: usize,
    #[arg(long, default_value_t = 128)]
    pub(super) max_nodes: usize,
    #[arg(long, default_value_t = 3)]
    pub(super) max_depth: usize,
    /// Optional exact graph node to anchor the clue search.
    #[arg(long)]
    pub(super) seed: Option<String>,
    /// Source role filter: user, assistant, or source.
    #[arg(long)]
    pub(super) role: Option<String>,
    /// Explicit native store override for the graph's typed conversation source.
    #[arg(long)]
    pub(super) source_root: Option<PathBuf>,
    #[arg(long)]
    pub(super) refresh: bool,
}

use std::path::PathBuf;

pub(super) fn cmd_trace(root: &std::path::Path, args: &TraceArgs) -> i32 {
    let result = if let Some(key) = &args.expand {
        innen_core::trace::expand(
            root,
            key,
            args.record.unwrap_or(0),
            args.expect_sha256.as_deref().unwrap_or(""),
        )
    } else {
        innen_core::trace::search(
            root,
            &innen_core::trace::Options {
                q: args.q.clone().unwrap_or_default(),
                limit: args.limit,
                max_sources: args.max_sources,
                max_bytes: args.max_bytes,
                max_records: args.max_records,
                max_nodes: args.max_nodes,
                max_depth: args.max_depth,
                seed: args.seed.clone(),
                role: args.role.clone(),
                source_root: args.source_root.clone(),
                refresh: args.refresh,
            },
        )
    };
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

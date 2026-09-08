use std::path::PathBuf;

#[derive(clap::Args)]
pub(in crate::cli) struct MiddlewareArgs {
    #[command(subcommand)]
    pub(super) op: MiddlewareOp,
}

#[derive(clap::Subcommand)]
pub(in crate::cli) enum MiddlewareOp {
    /// Native SessionStart adapter for a caller-pinned active task history.
    HistoryHook {
        #[arg(long, conflicts_with = "store", required_unless_present = "store")]
        file: Option<String>,
        /// Private persistent history store.
        #[arg(long, conflicts_with = "file", requires_all = ["project", "task"])]
        store: Option<PathBuf>,
        /// Exact project identity when loading from --store.
        #[arg(long, requires = "store")]
        project: Option<String>,
        /// Exact task identity when loading from --store.
        #[arg(long, requires = "store")]
        task: Option<String>,
        /// Exact workspace allowed to consume this task packet.
        #[arg(long)]
        cwd: PathBuf,
    },
    /// Select current decisions, causal history, and lessons for an exact task.
    HistoryPrepare {
        #[arg(long)]
        file: String,
        #[arg(long)]
        packet_only: bool,
    },
    /// Recover original history records against their full request hash.
    HistoryExpand {
        #[arg(long)]
        file: String,
        #[arg(long)]
        expect_source_sha256: String,
        #[arg(long, value_delimiter = ',', required = true)]
        ids: Vec<String>,
    },
    /// Save a validated task history envelope in a private append-only store.
    HistorySave {
        #[arg(long)]
        file: String,
        #[arg(long)]
        store: PathBuf,
    },
    /// Load the latest validated full envelope for an exact task.
    HistoryLoad {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: String,
    },
    /// Select caller-provided context without semantic inference or network access.
    Prepare {
        /// JSON request envelope. Use - to read stdin.
        #[arg(long)]
        file: String,
        /// Emit only the packet intended for a model request, without audit metadata.
        #[arg(long)]
        packet_only: bool,
    },
    /// Expand exact caller-selected items after verifying the source request hash.
    Expand {
        /// JSON request envelope. Use - to read stdin.
        #[arg(long)]
        file: String,
        /// Source hash returned by `middleware prepare`.
        #[arg(long)]
        expect_source_sha256: String,
        /// Comma-separated item IDs to expand.
        #[arg(long, value_delimiter = ',', required = true)]
        ids: Vec<String>,
    },
}

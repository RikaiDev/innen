use super::{
    artifact::ArtifactOp,
    checkpoint::CheckpointArgs,
    cloud::CloudOp,
    config::ConfigOp,
    conversation::ConversationArgs,
    graph::GraphOp,
    harvest::HarvestArgs,
    hook::HookArgs,
    index::IndexOp,
    middleware::MiddlewareArgs,
    navigation::{ProjectArgs, SearchArgs, TimelineArgs},
    pickup::PickupArgs,
    query::QueryArgs,
    resume::ResumeArgs,
    trace::TraceArgs,
    unfinished::UnfinishedArgs,
};

#[derive(clap::Subcommand)]
pub(super) enum Commands {
    /// Read a local conversation by UUID without ingesting it.
    #[command(visible_alias = "read")]
    Conversation(ConversationArgs),
    /// Resume a conversation into working context with compact factoring.
    Resume(ResumeArgs),
    /// Record, show, or list progress checkpoints in append-only storage outside Git.
    Checkpoint(CheckpointArgs),
    /// Discover recent unfinished candidate conversations across coding agents.
    Unfinished(UnfinishedArgs),
    /// Pick up an unambiguous unfinished conversation with exact continuation instructions.
    Pickup(PickupArgs),
    /// Ranked query over the journal (replay + FTS + graph BFS).
    Query(QueryArgs),
    /// Typed graph writes (journal appends).
    Graph {
        #[command(subcommand)]
        op: GraphOp,
    },
    /// Derived index maintenance.
    Index {
        #[command(subcommand)]
        op: IndexOp,
    },
    /// Report-only health checks (exit 0/1/2).
    Doctor,
    /// Report-only lint subset (exit 0/1/2).
    Lint,
    /// Persisted config get/set (machine.json layer).
    Config {
        #[command(subcommand)]
        op: ConfigOp,
    },
    /// Print shell completions to stdout.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_parser = ["bash", "zsh", "fish", "powershell"])]
        shell: String,
    },
    /// Short navigation text (Task 9 core `guide_text`).
    Guide,
    /// Lexical-only search over label/body (Task 9 core `search`).
    Search(SearchArgs),
    /// Journal counts (Task 9 core `status`).
    Status,
    /// Journal history in order (Task 9 core `timeline`).
    Timeline(TimelineArgs),
    /// What remains across projects, or in one project; expand evidence on demand.
    Project(ProjectArgs),
    /// Profile page render from profile/profile.toml (Task 10).
    Profile,
    /// Content-addressed artifact writes (Task 11a).
    Artifact {
        #[command(subcommand)]
        op: ArtifactOp,
    },
    /// Stubbed rclone cloud harness (Task 11b; `rclone` via PATH lookup).
    Cloud {
        #[command(subcommand)]
        op: CloudOp,
    },
    /// Dry-run harvest check over `00-inbox/harvest` (Task 14).
    Harvest(HarvestArgs),
    /// Agent stop hooks: install config, snapshot on stop, list pending.
    Hook(HookArgs),
    /// Ingest new inbox files into the journal (Task 14).
    Ingest,
    /// Prepare a bounded offline context packet from caller-selected JSON.
    Middleware(MiddlewareArgs),
    /// Trace knowledge clues through graph provenance to source records.
    Trace(TraceArgs),
    /// Synchronize wiki pages and their explicit source relationships.
    Wiki {
        #[command(subcommand)]
        op: WikiOp,
    },
}

#[derive(clap::Subcommand)]
pub(super) enum WikiOp {
    /// Append changed wiki metadata and managed source edges to the graph.
    Sync,
}

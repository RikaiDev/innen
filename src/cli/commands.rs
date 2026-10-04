use super::{
    artifact::ArtifactOp,
    checkpoint::CheckpointArgs,
    config::ConfigOp,
    conversation::ConversationArgs,
    graph::GraphOp,
    harvest::HarvestArgs,
    hook::HookArgs,
    index::IndexOp,
    middleware::MiddlewareArgs,
    navigation::{ProjectArgs, TimelineArgs},
    pickup::PickupArgs,
    query::QueryArgs,
    resume::ResumeArgs,
    retention::RetentionArgs,
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
    /// Short navigation text.
    Guide,
    /// Journal counts.
    Status,
    /// Journal history in order.
    Timeline(TimelineArgs),
    /// What remains across projects, or in one project; expand evidence on demand.
    Project(ProjectArgs),
    /// Profile page render from profile/profile.toml.
    Profile,
    /// Content-addressed artifact writes.
    Artifact {
        #[command(subcommand)]
        op: ArtifactOp,
    },
    /// Dry-run harvest check over `00-inbox/harvest`.
    Harvest(HarvestArgs),
    /// Agent stop hooks: install config, snapshot on stop, list pending.
    Hook(HookArgs),
    /// Ingest new inbox files into the journal.
    Ingest,
    /// Inventory, attest, and evidence-gate native coding-session retention.
    Retention(RetentionArgs),
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

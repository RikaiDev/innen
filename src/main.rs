//! P1+P2 CLI dispatch (Tasks 8d, 12).
//!
//! Thin dispatch: parse → core calls → print. No business logic here.
//!
//! KB root resolution (pinned): `--root <dir>` flag wins, else `INNEN_ROOT`
//! env (non-empty), else current working directory.
//!
//! Output format: global `--format human|json` (default `json`). Explicit
//! only — no TTY sniffing, no env fallback for rendering. Deterministic:
//! `--format json` is compact `serde_json::to_string` (single line +
//! trailing newline); struct field order is the wire order.
//!
//! P2 commands (Task 12): `guide`, `search --keyword --limit`, `status`,
//! `timeline [filter]`, `project <id>`, `profile`, `artifact add --file`,
//! `cloud status|doctor` (Task 14 adds `harvest --check`, `ingest`). P2 JSON shapes (field order is wire order):
//! guide `{"text"}`, search `{"hits":[{node_id,excerpt}]}`,
//! status `{"nodes","edges","events","artifacts"}`,
//! timeline `{"entries":[{observed_utc,op,summary}]}`,
//! project `{"id","render"}`, profile `{"render"}`,
//! artifact `{"sha256","path","bytes"}`,
//! cloud status `{"remote","output"}`, cloud doctor `{"output"}`,
//! harvest check `{"taps":[{id,new_files,skipped}]}`,
//! ingest `{"added","skipped":[{path,pattern,preview}]}`.
//! P2 human renders raw markdown for project/profile (byte-exact via
//! `print!`, no extra newline) and TSV tables elsewhere; P2 errors print
//! the core message raw to stderr (no `error:` prefix) so
//! `unknown project: <id>` matches byte-for-byte.
//! NOTE (P1/P2 error prefix, intentional): P1 arms print `error: {e}`;
//! P2 arms (`project`/`profile`/`artifact`/`cloud`) keep raw `{e}` per the
//! plan-pinned literal `unknown project: <id>`. Do not "unify" P2 to
//! `error:` — that would break the plan literal and byte-exact stderr.

use std::path::PathBuf;

use clap::{CommandFactory as _, Parser, Subcommand};

/// KB root resolution: `--root` flag > `INNEN_ROOT` env (non-empty) > user-level global config.
/// Fails with an actionable error if unconfigured. Never silently falls back to cwd.
fn resolve_root(
    cli_root: Option<PathBuf>,
) -> Result<PathBuf, innen_core::config::RootResolutionError> {
    innen_core::config::resolve_root(cli_root)
}

#[derive(Parser)]
#[command(
    name = "innen",
    version,
    about = "innen P1+P2 knowledge CLI",
    long_about = "innen P1+P2 knowledge CLI.\n\nKB root resolution: --root <dir> > INNEN_ROOT env (non-empty) > user-level persisted root (~/.config/innen/config.json).\n\nOperational boundaries:\n  harvest: imports conversation transcripts into knowledge graph entities\n  checkpoint: records progress snapshots in append-only storage outside Git\n  unfinished: discovers candidate unfinished conversations using structural evidence\n  resume / pickup: reconstructs working context and provides continuation instructions"
)]
struct Cli {
    /// Output format (explicit only; no TTY sniffing). Applies to all commands.
    #[arg(long, global = true, default_value = "json", value_parser = ["json", "human"])]
    format: String,
    /// KB root dir. Precedence: --root > INNEN_ROOT env > global config.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
    /// Ingest new inbox files into the journal (Task 14).
    Ingest,
}

#[derive(clap::Args)]
struct ConversationArgs {
    /// UUID or native session ID (for example OpenCode ses_...).
    #[arg(required_unless_present_any = ["decode_packet", "validate_packet", "encode_json"])]
    uuid: Option<String>,
    /// Validate and restore a saved self-describing conversation packet, offline.
    #[arg(long, conflicts_with_all = ["uuid", "validate_packet", "encode_json"])]
    decode_packet: Option<PathBuf>,
    /// Validate a saved packet and emit it unchanged for model input, offline.
    #[arg(long, conflicts_with_all = ["uuid", "decode_packet", "encode_json"])]
    validate_packet: Option<PathBuf>,
    /// Encode a saved ordinary conversation JSON page, offline.
    #[arg(long, conflicts_with_all = ["uuid", "decode_packet", "validate_packet"])]
    encode_json: Option<PathBuf>,
    /// Opt-in reference-token-selected conversation grammar; legacy remains default.
    #[arg(long, default_value = "legacy", value_parser = ["legacy", "conversation"])]
    codec: String,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to read; auto searches standard local stores and rejects ambiguity.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Dialogue is text; events retains native fields; index is incomplete navigation previews.
    #[arg(long, default_value = "dialogue", value_parser = ["dialogue", "events", "index", "context"])]
    view: String,
    /// Factor repeated event fields without summarizing. Falls back if bytes grow.
    #[arg(long)]
    compact: bool,
    /// Experimentally share exact string prefixes/suffixes; no semantic edits.
    #[arg(long, requires = "compact")]
    deltas: bool,
    /// Reference image data URIs and known encrypted fields. Requires events view.
    #[arg(long, conflicts_with = "attachment")]
    attachment_refs: bool,
    /// Retrieve a source string at this JSON pointer; uses events view and limit 1.
    #[arg(long, requires = "expect_sha256", conflicts_with = "compact")]
    attachment: Option<String>,
    /// Expected UTF-8 string hash from an attachment reference; fail on changes.
    #[arg(long, requires = "attachment")]
    expect_sha256: Option<String>,
    /// Batch exact one-based source lines (comma-separated); returns raw events in source order.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["offset", "limit", "attachment"])]
    lines: Vec<usize>,
    /// Zero-based physical JSONL row; use next_offset from the previous page.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Maximum matching events per page (1..100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
}

#[derive(clap::Args)]
struct ResumeArgs {
    /// UUID or native session ID. If omitted, inspects project metadata for an unambiguous candidate.
    uuid: Option<String>,
    /// Project root directory. Defaults to the current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to read; auto searches standard local stores and rejects ambiguity.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Projection view: brief (default), context, dialogue, events, index.
    #[arg(long, value_parser = ["brief", "dialogue", "events", "index", "context"])]
    view: Option<String>,
    /// Omit compact factoring (compact is enabled by default for resume).
    #[arg(long)]
    no_compact: bool,
    /// Omit delta prefix/suffix sharing (deltas is enabled by default for resume).
    #[arg(long)]
    no_deltas: bool,
    /// Reference image data URIs and known encrypted fields. Requires events view.
    #[arg(long, conflicts_with = "attachment")]
    attachment_refs: bool,
    /// Retrieve a source string at this JSON pointer; uses events view and limit 1.
    #[arg(long, requires = "expect_sha256")]
    attachment: Option<String>,
    /// Expected UTF-8 string hash from an attachment reference; fail on changes.
    #[arg(long, requires = "attachment")]
    expect_sha256: Option<String>,
    /// Batch exact one-based source lines (comma-separated); returns raw events in source order.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["offset", "limit", "attachment"])]
    lines: Vec<usize>,
    /// Zero-based physical JSONL row; use next_offset from the previous page.
    #[arg(long)]
    offset: Option<usize>,
    /// Maximum matching events per page (1..100).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: Option<u16>,
}

#[derive(clap::Args)]
struct CheckpointArgs {
    #[command(subcommand)]
    op: Option<CheckpointOp>,
    /// Session UUID or native ID (when no subcommand provided).
    #[arg(long)]
    session: Option<String>,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(Subcommand)]
enum CheckpointOp {
    /// Record or update a progress checkpoint (append-only history).
    Record(Box<CheckpointRecordArgs>),
    /// Show the latest checkpoint for a session.
    Show(CheckpointShowArgs),
    /// List latest checkpoints for the project.
    List(CheckpointListArgs),
}

#[derive(clap::Args)]
struct CheckpointRecordArgs {
    /// Session UUID or native ID. If omitted, checks project metadata for an unambiguous candidate.
    #[arg(long)]
    session: Option<String>,
    /// Tool source (auto, agy, codex, ...).
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Checkpoint status: active, blocked, completed, superseded.
    #[arg(long, default_value = "active", value_parser = ["active", "blocked", "completed", "superseded"])]
    status: String,
    /// High-level goal or objective.
    #[arg(long)]
    objective: Option<String>,
    /// Completed work items (comma-separated).
    #[arg(long, value_delimiter = ',')]
    completed: Vec<String>,
    /// Current evidence / observations (comma-separated).
    #[arg(long, value_delimiter = ',')]
    evidence: Vec<String>,
    /// Known blockers or open questions (comma-separated).
    #[arg(long, value_delimiter = ',')]
    blocker: Vec<String>,
    /// Immediate next action for continuation.
    #[arg(long)]
    next_action: Option<String>,
    /// Verification command or criteria.
    #[arg(long)]
    verification: Option<String>,
    /// Load checkpoint JSON from file or '-' for stdin.
    #[arg(long)]
    file: Option<String>,
}

#[derive(clap::Args)]
struct CheckpointShowArgs {
    /// Session UUID or native ID. If omitted, checks project metadata for an unambiguous candidate.
    session: Option<String>,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(clap::Args)]
struct CheckpointListArgs {
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(clap::Args)]
struct UnfinishedArgs {
    /// Discover unfinished candidates across all projects (portfolio discovery).
    #[arg(long, conflicts_with = "project")]
    all_projects: bool,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to filter (auto, agy, codex, ...).
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Time cutoff (default yesterday+today; YYYY-MM-DD, RFC3339, 2d, 48h, today, yesterday).
    #[arg(long)]
    since: Option<String>,
}

#[derive(clap::Args)]
struct PickupArgs {
    /// Discover candidates across all projects (portfolio discovery).
    #[arg(long, conflicts_with = "project")]
    all_projects: bool,
    /// Optional UUID or native session ID. If omitted, discovers recent unfinished candidate.
    uuid: Option<String>,
    /// Project root directory. Defaults to current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to filter (auto, agy, codex, ...).
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Time cutoff (default yesterday+today; YYYY-MM-DD, RFC3339, 2d, 48h, today, yesterday).
    #[arg(long)]
    since: Option<String>,
}

#[derive(clap::Args)]
struct QueryArgs {
    /// Query string (natural text or literal identifier/filename).
    #[arg(long)]
    q: String,
    /// Evaluate pinned fact alternatives and dependency closure against the journal.
    #[arg(long, conflicts_with_all = ["as_of", "include_expired", "view", "limit", "offset"])]
    evidence_contract: Option<PathBuf>,
    /// Prepare inspected source context for an external model; never invokes one.
    #[arg(
        long,
        requires = "evidence_contract",
        conflicts_with = "proposal_input"
    )]
    prepare_proposal: bool,
    /// Validate an external model's cited task proposal against the trusted scope.
    #[arg(long, requires = "evidence_contract", group = "proposal_input")]
    task_proposal: Option<PathBuf>,
    /// Validate ID-only output inside a controller-bound context envelope.
    #[arg(long, requires = "evidence_contract", group = "proposal_input")]
    task_draft: Option<PathBuf>,
    /// Prepare source IDs and a native model output schema.
    #[arg(long, requires = "prepare_proposal")]
    evidence_ids: bool,
    /// Caller review receipt bound to exact context and proposal hashes.
    #[arg(long, requires = "proposal_input")]
    proposal_review: Option<PathBuf>,
    /// As-of cutoff `YYYY-MM-DDTHH:MM:SSZ` (defaults to now).
    #[arg(long = "as-of")]
    as_of: Option<String>,
    /// Max hits (default 20, max 100; >100 clamps with `truncated`).
    #[arg(long, default_value_t = 20)]
    limit: u16,
    /// Zero-based task/event offset for pagination.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Bypass validity filtering (keep expired edges).
    #[arg(long = "include-expired")]
    include_expired: bool,
    /// View mode: context (default compact task-context brief), hits (legacy FTS+graph hits), or evidence (full payload + physical history).
    #[arg(long, default_value = "context", value_parser = ["context", "hits", "evidence"])]
    view: String,
}

#[derive(Subcommand)]
enum GraphOp {
    /// Append a node.upsert event.
    Node(GraphNodeArgs),
    /// Append an edge.assert event (adjacency-validated).
    Relate(GraphRelateArgs),
    /// Append an edge.retract event.
    Retract(GraphRetractArgs),
}

#[derive(clap::Args)]
struct GraphNodeArgs {
    /// Node id.
    #[arg(long)]
    id: String,
    /// Node kind (NodeType display name; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    kind: String,
    /// Human label.
    #[arg(long)]
    label: String,
    /// Optional body text.
    #[arg(long)]
    body: Option<String>,
    /// Provenance (required when kind is custom).
    #[arg(long)]
    provenance: Option<String>,
}

#[derive(clap::Args)]
struct GraphRelateArgs {
    /// From node id.
    #[arg(long)]
    from: String,
    /// Edge type (SCREAMING_SNAKE_CASE; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    edge: String,
    /// To node id or URI (`/abs` or `scheme://...` only under LOCATED_AT/ORIGINATED_AT).
    #[arg(long)]
    to: String,
    /// Optional weight.
    #[arg(long)]
    weight: Option<f32>,
    /// Optional valid-from `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-from")]
    valid_from: Option<String>,
    /// Optional valid-until `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-until")]
    valid_until: Option<String>,
    /// Provenance (required for custom parties).
    #[arg(long)]
    provenance: Option<String>,
}

#[derive(clap::Args)]
struct GraphRetractArgs {
    /// From node id.
    #[arg(long)]
    from: String,
    /// Edge type.
    #[arg(long, visible_alias = "type")]
    edge: String,
    /// To node id.
    #[arg(long)]
    to: String,
}

#[derive(Subcommand)]
enum IndexOp {
    /// Drop derived files and rebuild from the journal.
    Rebuild,
}

#[derive(Subcommand)]
enum ConfigOp {
    /// Print one resolved config value as JSON.
    Get {
        /// Key: root | format | rebuild_on_open.
        key: String,
        /// Read from user-level global config (~/.config/innen/config.json).
        #[arg(long)]
        global: bool,
    },
    /// Persist one key into .innen/machine.json (or global config with --global).
    Set {
        /// Key: root | format | rebuild_on_open.
        key: String,
        /// Value to store.
        value: String,
        /// Persist to user-level global config (~/.config/innen/config.json).
        #[arg(long)]
        global: bool,
    },
}

/// P2 `harvest --check`: dry-run only (no appends, no watermark advance).
#[derive(clap::Args)]
struct HarvestArgs {
    /// Dry-run flag (accepted for compat; harvest is always a dry-run check).
    #[arg(long, default_value_t = false)]
    check: bool,
}

/// P2 `search --keyword <kw> [--limit <n>]`: lexical-only, no graph walk.
#[derive(clap::Args)]
struct SearchArgs {
    /// Keyword (tantivy query over label/body; empty matches nothing).
    #[arg(long)]
    keyword: String,
    /// Max hits (default 20).
    #[arg(long, default_value_t = 20)]
    limit: u16,
}

/// P2 `timeline [filter]`: journal order; filter is a `YYYY[-MM]` prefix.
#[derive(clap::Args)]
struct TimelineArgs {
    /// Optional prefix filter, e.g. `2026-09`.
    filter: Option<String>,
}

/// P2 `project <id>`: project page render.
#[derive(clap::Args)]
struct ProjectArgs {
    /// Project id or slug. Omit for the portfolio of recorded tasks.
    id: Option<String>,
    /// Brief pending tasks (default), evidence with history, or legacy full project page.
    #[arg(long, default_value = "brief", value_parser = ["brief", "evidence", "full"])]
    view: String,
    /// Maximum task rows, with an explicit next_offset when more remain.
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=1000))]
    limit: u16,
    /// Zero-based task offset for the next page.
    #[arg(long, default_value_t = 0)]
    offset: usize,
}

#[derive(Subcommand)]
enum ArtifactOp {
    /// Store a file content-addressed + record artifact node/edge.
    Add(ArtifactAddArgs),
    /// Inventory a directory and archive its hash-bound metadata manifest.
    AddTree(ArtifactAddTreeArgs),
    /// Maintain a project archive ledger without copying source bytes.
    Ledger {
        #[command(subcommand)]
        op: Box<LedgerOp>,
    },
}

#[derive(Subcommand)]
enum LedgerOp {
    /// Initialize or reopen an exact-title archive root.
    Init(LedgerInitArgs),
    /// Register a source receipt and mirror its typed identity into the KB journal.
    Add(LedgerAddArgs),
    /// Append a note, supersession, hash-verified relocation, or reclassification event.
    Event(LedgerEventArgs),
    /// Check source hashes and lifecycle state.
    Check(LedgerCheckArgs),
}

#[derive(clap::Args)]
struct LedgerInitArgs {
    #[arg(long = "archive-root")]
    archive_root: PathBuf,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    title: String,
    #[arg(long)]
    downloads_root: Option<PathBuf>,
    #[arg(long, value_delimiter = ',')]
    categories: Vec<String>,
}

#[derive(clap::Args)]
struct LedgerAddArgs {
    #[arg(long = "archive-root")]
    archive_root: PathBuf,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    source_kind: String,
    #[arg(long)]
    role: String,
    #[arg(long)]
    stage: String,
    #[arg(long)]
    owner_project: String,
    /// Archive project identity (A); owner_project may identify a separate
    /// project (B) for a reference receipt.
    #[arg(long)]
    archive_project_id: Option<String>,
    #[arg(long)]
    relation: String,
    #[arg(long)]
    document_date: Option<String>,
    #[arg(long)]
    authority: Option<String>,
    #[arg(long)]
    evidence_status: Option<String>,
    #[arg(long)]
    document_id: Option<String>,
    #[arg(long)]
    approval_valid_from: Option<String>,
    #[arg(long)]
    approval_valid_until: Option<String>,
    #[arg(long)]
    provenance: String,
    #[arg(long)]
    downloads_root: Option<PathBuf>,
}

#[derive(clap::Args)]
struct LedgerCheckArgs {
    #[arg(long = "archive-root")]
    archive_root: PathBuf,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    title: String,
}

#[derive(clap::Args)]
struct LedgerEventArgs {
    #[arg(long = "archive-root")]
    archive_root: PathBuf,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    title: String,
    #[arg(long)]
    kind: String,
    #[arg(long)]
    receipt_id: String,
    #[arg(long)]
    superseded_receipt_id: Option<String>,
    #[arg(long)]
    new_path: Option<PathBuf>,
    #[arg(long)]
    expected_sha256: Option<String>,
    #[arg(long)]
    event_date: Option<String>,
    #[arg(long)]
    note: Option<String>,
    /// New owner project for a reclassification; omitted keeps the recorded owner.
    #[arg(long)]
    new_owner_project: Option<String>,
    /// New relation (`belongs_to` or `reference`) for a reclassification.
    #[arg(long)]
    new_relation: Option<String>,
    /// Evidence explaining a reclassification (required for that event kind).
    #[arg(long)]
    provenance: Option<String>,
    /// Archive project identity to retain when mirroring a lifecycle event.
    #[arg(long)]
    archive_project_id: Option<String>,
}

/// P2 `artifact add --file <path> [--project <id>]`.
#[derive(clap::Args)]
struct ArtifactAddArgs {
    /// Source file to store.
    #[arg(long)]
    file: PathBuf,
    /// Optional project id for a BELONGS_TO edge.
    #[arg(long)]
    project: Option<String>,
}

#[derive(clap::Args)]
struct ArtifactAddTreeArgs {
    /// Directory to inventory without following symlinks.
    #[arg(long)]
    directory: PathBuf,
    /// Manifest output path outside the inventoried directory.
    #[arg(long)]
    manifest: PathBuf,
    /// Optional project id for a BELONGS_TO edge.
    #[arg(long)]
    project: Option<String>,
}

#[derive(Subcommand)]
enum CloudOp {
    /// `rclone lsd <remote>:` (canned `canned-dir` under the test stub).
    Status(CloudStatusArgs),
    /// `rclone version` (stub reports canned version).
    Doctor,
}

/// P2 `cloud status [--remote <name>]` (default `myremote` so bare
/// `cloud status` works against the stub).
#[derive(clap::Args)]
struct CloudStatusArgs {
    /// Remote name (stub ignores the value).
    #[arg(long, default_value = "myremote")]
    remote: String,
}

// --- P2 JSON wire structs (field order is the wire order) ---

#[derive(serde::Serialize)]
struct GuideJson {
    text: String,
}

#[derive(serde::Serialize)]
struct SearchHitJson {
    node_id: String,
    excerpt: String,
}

#[derive(serde::Serialize)]
struct SearchJson {
    hits: Vec<SearchHitJson>,
}

#[derive(serde::Serialize)]
struct StatusJson {
    nodes: u64,
    edges: u64,
    events: u64,
    artifacts: u64,
}

#[derive(serde::Serialize)]
struct TimelineEntryJson {
    observed_utc: String,
    op: String,
    summary: String,
}

#[derive(serde::Serialize)]
struct TimelineJson {
    entries: Vec<TimelineEntryJson>,
}

#[derive(serde::Serialize)]
struct ProjectJson {
    id: String,
    render: String,
}

#[derive(serde::Serialize)]
struct ProfileJson {
    render: String,
}

#[derive(serde::Serialize)]
struct ArtifactJson {
    sha256: String,
    path: String,
    bytes: u64,
}

#[derive(serde::Serialize)]
struct ArtifactTreeJson {
    tree_sha256: String,
    files: u64,
    symlinks: u64,
    source_bytes: u64,
    manifest_sha256: String,
    manifest_path: String,
    stored_path: String,
    metadata_only: bool,
}

#[derive(serde::Serialize)]
struct CloudStatusJson {
    remote: String,
    output: String,
}

#[derive(serde::Serialize)]
struct CloudDoctorJson {
    output: String,
}

fn is_human(format: &str) -> bool {
    format == "human"
}

/// TSV field budget (chars, not bytes — CJK safe) for human tables.
const TSV_FIELD_LIMIT: usize = 200;

/// Escape one human-table TSV field: truncate to [`TSV_FIELD_LIMIT`] chars
/// (CJK-safe, chars not bytes) with a `…` marker, then encode `\t`→`\\t`,
/// `\n`→`\\n`, `\r`→`\\r` so embedded tabs/newlines cannot break rows.
/// Truncation runs first so escape sequences stay intact.
fn escape_tsv_field(s: &str) -> String {
    let truncated: String = if s.chars().count() > TSV_FIELD_LIMIT {
        let mut out: String = s.chars().take(TSV_FIELD_LIMIT).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    };
    truncated
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn print_query_json(out: &innen_core::query::QueryOutput) {
    println!(
        "{}",
        serde_json::to_string(out).expect("query output serializes")
    );
}

fn print_query_human(out: &innen_core::query::QueryOutput) {
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

fn print_report_json(report: &innen_core::doctor::Report) {
    println!(
        "{}",
        serde_json::to_string(report).expect("report serializes")
    );
}

fn print_report_human(report: &innen_core::doctor::Report) {
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

fn cmd_query(root: &std::path::Path, format: &str, args: &QueryArgs) -> i32 {
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

fn cmd_graph_node(root: &std::path::Path, format: &str, args: &GraphNodeArgs) -> i32 {
    use serde_json::json;
    // Thin call: custom-provenance rule lives in innen-core::graph.
    let parsed: innen_core::graph::NodeType = args.kind.parse().unwrap();
    if let Err(e) = innen_core::graph::check_node_provenance(&parsed, args.provenance.as_deref()) {
        eprintln!("error: {e}");
        return 1;
    }
    let mut payload = json!({
        "id": args.id,
        "type": args.kind,
        "label": args.label,
    });
    if let Some(body) = &args.body {
        payload["body"] = json!(body);
    }
    if let Some(prov) = &args.provenance {
        payload["provenance"] = json!(prov);
    }
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("node.upsert", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("created\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("node output serializes")
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

fn cmd_graph_relate(root: &std::path::Path, format: &str, args: &GraphRelateArgs) -> i32 {
    use serde_json::json;
    let edge_ty: innen_core::graph::EdgeType = args.edge.parse().unwrap();
    // Thin call: endpoint-kind resolution + adjacency/provenance validation
    // lives in innen-core::graph (best-effort open may quarantine; that is the
    // normal write-path behavior for graph mutations).
    if let Err(e) = innen_core::graph::validate_relate_request(
        root,
        &args.from,
        &edge_ty,
        &args.to,
        args.provenance.as_deref(),
    ) {
        eprintln!("error: {e}");
        return 1;
    }
    let mut payload = json!({
        "from": args.from,
        "type": edge_ty.to_string(),
        "to": args.to,
    });
    if let Some(w) = args.weight {
        payload["weight"] = json!(w);
    }
    if let Some(vf) = &args.valid_from {
        payload["valid_from"] = json!(vf);
    }
    if let Some(vu) = &args.valid_until {
        payload["valid_until"] = json!(vu);
    }
    if let Some(prov) = &args.provenance {
        payload["provenance"] = json!(prov);
    }
    // Re-open is cheap; reuse open handle would self-block on the fs2 lock,
    // so open once more here for the append (sequential, not nested).
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("edge.assert", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("created\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("relate output serializes")
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

fn cmd_graph_retract(root: &std::path::Path, format: &str, args: &GraphRetractArgs) -> i32 {
    let edge_ty: innen_core::graph::EdgeType = args.edge.parse().unwrap();
    let payload = serde_json::json!({
        "from": args.from,
        "type": edge_ty.to_string(),
        "to": args.to,
    });
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("edge.retract", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("retracted\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("retract output serializes")
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

fn cmd_index_rebuild(root: &std::path::Path, format: &str) -> i32 {
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

fn cmd_doctor(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::run(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

fn cmd_lint(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::lint(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

fn config_value(cfg: &innen_core::config::Config, key: &str) -> Option<String> {
    match key {
        "root" => Some(cfg.root.to_string_lossy().into_owned()),
        "format" => Some(cfg.format.clone()),
        "rebuild_on_open" => Some(cfg.rebuild_on_open.to_string()),
        _ => None,
    }
}

fn cmd_config_get(root: &std::path::Path, format: &str, key: &str) -> i32 {
    // Resolved config (env > machine.json > innen.toml > builtin). The global
    // --format rendering flag is intentionally NOT a config override here:
    // `config get format` reports the persisted value.
    let (cfg, warnings) =
        innen_core::config::load(root, &innen_core::config::CliOverrides::default());
    for w in &warnings {
        eprintln!("{w}");
    }
    let Some(value) = config_value(&cfg, key) else {
        eprintln!("error: unknown config key: {key} (root|format|rebuild_on_open)");
        return 1;
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&value));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": value}))
                .expect("config output serializes")
        );
    }
    0
}

fn cmd_config_set(root: &std::path::Path, format: &str, key: &str, value: &str) -> i32 {
    // Thin call: validation + machine.json persistence lives in innen-core::config.
    let echo = match innen_core::config::set_machine_value(root, key, value) {
        Ok(echo) => echo,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&echo));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
}

fn cmd_config_get_global(format: &str, key: &str) -> i32 {
    let value = match innen_core::config::get_global_value(key) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!("error: key '{key}' is not set in global config");
            return 1;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&value));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": value}))
                .expect("config output serializes")
        );
    }
    0
}

fn cmd_config_set_global(format: &str, key: &str, value: &str) -> i32 {
    let echo = match innen_core::config::set_global_value(key, value) {
        Ok(echo) => echo,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&echo));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
}

/// Resolve the `rclone` binary via PATH lookup (CLI-only; unit tests
/// inject the fixture path directly into `Rclone { bin }`).
fn resolve_rclone_bin() -> Result<PathBuf, String> {
    let path_var =
        std::env::var_os("PATH").ok_or_else(|| "rclone not found in PATH".to_string())?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join("rclone");
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err("rclone not found in PATH".to_string())
}

fn cmd_guide(format: &str) -> i32 {
    let text = innen_core::parity::guide_text();
    if is_human(format) {
        println!("{text}");
    } else {
        println!(
            "{}",
            serde_json::to_string(&GuideJson {
                text: text.to_string(),
            })
            .expect("guide output serializes")
        );
    }
    0
}

fn cmd_search(root: &std::path::Path, format: &str, args: &SearchArgs) -> i32 {
    let hits = innen_core::parity::search(root, &args.keyword, args.limit);
    if is_human(format) {
        println!("node_id\texcerpt");
        for h in &hits {
            println!(
                "{}\t{}",
                escape_tsv_field(&h.node_id),
                escape_tsv_field(&h.excerpt)
            );
        }
    } else {
        let out = SearchJson {
            hits: hits
                .into_iter()
                .map(|h| SearchHitJson {
                    node_id: h.node_id,
                    excerpt: h.excerpt,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("search output serializes")
        );
    }
    0
}

fn cmd_status(root: &std::path::Path, format: &str) -> i32 {
    let s = innen_core::parity::status(root);
    if is_human(format) {
        println!("nodes\tedges\tevents\tartifacts");
        println!("{}\t{}\t{}\t{}", s.nodes, s.edges, s.events, s.artifacts);
    } else {
        let out = StatusJson {
            nodes: s.nodes,
            edges: s.edges,
            events: s.events,
            artifacts: s.artifacts,
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("status output serializes")
        );
    }
    0
}

fn cmd_timeline(root: &std::path::Path, format: &str, args: &TimelineArgs) -> i32 {
    let rows = innen_core::parity::timeline(root, args.filter.as_deref());
    if is_human(format) {
        println!("observed_utc\top\tsummary");
        for e in &rows {
            println!(
                "{}\t{}\t{}",
                escape_tsv_field(&e.observed_utc),
                escape_tsv_field(&e.op),
                escape_tsv_field(&e.summary)
            );
        }
    } else {
        let out = TimelineJson {
            entries: rows
                .into_iter()
                .map(|e| TimelineEntryJson {
                    observed_utc: e.observed_utc,
                    op: e.op,
                    summary: e.summary,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("timeline output serializes")
        );
    }
    0
}

fn cmd_project(root: &std::path::Path, format: &str, id: &str) -> i32 {
    match innen_core::parity::project_render(root, id) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProjectJson {
                        id: id.to_string(),
                        render,
                    })
                    .expect("project output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_profile(root: &std::path::Path, format: &str) -> i32 {
    match innen_core::parity::profile_render(root) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProfileJson { render })
                        .expect("profile output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_artifact_add(root: &std::path::Path, format: &str, args: &ArtifactAddArgs) -> i32 {
    match innen_core::artifact::add(root, &args.file, args.project.as_deref()) {
        Ok(r) => {
            if is_human(format) {
                println!("sha256\tpath\tbytes");
                println!(
                    "{}\t{}\t{}",
                    escape_tsv_field(&r.sha256),
                    escape_tsv_field(&r.stored_path.to_string_lossy()),
                    r.bytes
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ArtifactJson {
                        sha256: r.sha256,
                        path: r.stored_path.to_string_lossy().into_owned(),
                        bytes: r.bytes,
                    })
                    .expect("artifact output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_artifact_add_tree(root: &std::path::Path, format: &str, args: &ArtifactAddTreeArgs) -> i32 {
    match innen_core::artifact::add_tree(
        root,
        &args.directory,
        &args.manifest,
        args.project.as_deref(),
    ) {
        Ok((tree, receipt)) => {
            let output = ArtifactTreeJson {
                tree_sha256: tree.tree_sha256,
                files: tree.files,
                symlinks: tree.symlinks,
                source_bytes: tree.bytes,
                manifest_sha256: receipt.sha256,
                manifest_path: args.manifest.to_string_lossy().into_owned(),
                stored_path: receipt.stored_path.to_string_lossy().into_owned(),
                metadata_only: true,
            };
            if is_human(format) {
                println!(
                    "tree_sha256\tfiles\tsymlinks\tsource_bytes\tmanifest_path\tmetadata_only"
                );
                println!(
                    "{}\t{}\t{}\t{}\t{}\ttrue",
                    output.tree_sha256,
                    output.files,
                    output.symlinks,
                    output.source_bytes,
                    escape_tsv_field(&output.manifest_path)
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&output).expect("artifact tree output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn ledger_source_kind(value: &str) -> Result<innen_core::artifact::ledger::SourceKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "document" => Ok(innen_core::artifact::ledger::SourceKind::Document),
        "event" => Ok(innen_core::artifact::ledger::SourceKind::Event),
        "dataset" => Ok(innen_core::artifact::ledger::SourceKind::Dataset),
        "other" => Ok(innen_core::artifact::ledger::SourceKind::Other),
        _ => Err(format!("invalid ledger source kind: {value}")),
    }
}

fn ledger_stage(value: &str) -> Result<innen_core::artifact::ledger::LifecycleStage, String> {
    match value.to_ascii_lowercase().as_str() {
        "intake" => Ok(innen_core::artifact::ledger::LifecycleStage::Intake),
        "authoring" => Ok(innen_core::artifact::ledger::LifecycleStage::Authoring),
        "verification" => Ok(innen_core::artifact::ledger::LifecycleStage::Verification),
        "delivery" => Ok(innen_core::artifact::ledger::LifecycleStage::Delivery),
        _ => Err(format!("invalid ledger lifecycle stage: {value}")),
    }
}

fn existing_ledger_project_id(root: &std::path::Path) -> Result<Option<String>, String> {
    let manifest = root.join(".innen-ledger/manifest.json");
    if !manifest.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&manifest).map_err(|e| format!("read ledger manifest: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse ledger manifest: {e}"))?;
    value
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "ledger manifest has no project_id".to_string())
        .map(Some)
}

fn mirror_ledger_receipt(
    kb_root: &std::path::Path,
    receipt: &innen_core::artifact::ledger::Receipt,
    archive_project: Option<&str>,
) -> Result<(), String> {
    let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
    // A content hash identifies bytes, while a receipt identifies this
    // immutable revision in the archive. Keep the latter in the graph so two
    // receipts with identical bytes cannot collapse into one revision node.
    let id = format!("artifact-revision:{}", receipt.id);
    let payload = serde_json::json!({
        "id": id,
        "type": "Artifact",
        "label": std::path::Path::new(&receipt.path)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("artifact"),
        "path": receipt.path,
        "sha256": receipt.sha256,
        "bytes": receipt.bytes,
        "status": format!("{:?}", receipt.stage).to_ascii_lowercase(),
        "owner_project": receipt.owner_project,
        "relation": receipt.relation,
        "authority": receipt.authority,
        "evidence_status": receipt.evidence_status,
        "provenance": receipt.provenance,
        "document_id": receipt.document_id,
        "revision_id": receipt.id,
        "approval_valid_from": receipt.approval_valid_from,
        "approval_valid_until": receipt.approval_valid_until,
        "archive_project": archive_project,
    });
    journal
        .append("node.upsert", &payload)
        .map_err(|e| format!("ledger receipt appended but KB node sync failed: {e}"))?;
    if let Some(document_id) = receipt.document_id.as_deref() {
        journal
            .append(
                "node.upsert",
                &serde_json::json!({
                    "id": document_id,
                    "type": "Document",
                    "label": document_id,
                    "archive_project": archive_project,
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| {
                format!("ledger artifact synced but document identity sync failed: {e}")
            })?;
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": document_id,
                    "type": "REVISION_OF",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger document synced but revision link failed: {e}"))?;
    }
    if receipt.relation == "belongs_to" && !receipt.owner_project.trim().is_empty() {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": receipt.owner_project,
                    "type": "BELONGS_TO",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger node synced but KB membership sync failed: {e}"))?;
    }
    if receipt.relation == "reference" && !receipt.owner_project.trim().is_empty() {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": receipt.owner_project,
                    "type": "REFERENCES_PROJECT",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger node synced but owner reference sync failed: {e}"))?;
    }
    if let Some(archive_project) = archive_project
        .filter(|archive| !archive.trim().is_empty())
        .filter(|archive| *archive != receipt.owner_project)
    {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": archive_project,
                    "type": "REFERENCES_PROJECT",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger owner synced but archive reference sync failed: {e}"))?;
    }
    Ok(())
}

fn mirror_ledger_event(
    kb_root: &std::path::Path,
    previous: &innen_core::artifact::ledger::Receipt,
    current: &innen_core::artifact::ledger::Receipt,
    event: &innen_core::artifact::ledger::EventRecord,
    archive_project: Option<&str>,
) -> Result<(), String> {
    let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
    let revision_id = format!("artifact-revision:{}", current.id);
    let retained_archive = archive_project.map(str::to_owned).or_else(|| {
        std::fs::read_to_string(kb_root.join(".innen/journal.jsonl"))
            .ok()?
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event.get("op").and_then(|v| v.as_str()) == Some("node.upsert"))
            .filter_map(|event| event.get("payload").cloned())
            .filter(|payload| payload.get("id").and_then(|v| v.as_str()) == Some(&revision_id))
            .filter_map(|payload| {
                payload
                    .get("archive_project")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .next_back()
    });
    let event_id = format!(
        "artifact-ledger-event:{}:{}",
        event.receipt_id, event.sequence
    );
    journal
        .append(
            "node.upsert",
            &serde_json::json!({
                "id": event_id,
                "type": "ArtifactLedgerEvent",
                "label": format!("{} {}", event.kind, event.receipt_id),
                "receipt_id": event.receipt_id,
                "event_date": event.event_date,
                "new_owner_project": event.new_owner_project,
                "new_relation": event.new_relation,
                "provenance": event.provenance,
            }),
        )
        .map_err(|e| format!("ledger event appended but KB event sync failed: {e}"))?;
    if event.kind == "reclassify" {
        let old_edge = if previous.relation == "belongs_to" {
            "BELONGS_TO"
        } else {
            "REFERENCES_PROJECT"
        };
        if !previous.owner_project.trim().is_empty() {
            journal
                .append(
                    "edge.retract",
                    &serde_json::json!({
                        "from": format!("artifact-revision:{}", previous.id),
                        "to": previous.owner_project,
                        "type": old_edge,
                    }),
                )
                .map_err(|e| format!("ledger event synced but old edge retract failed: {e}"))?;
        }
        mirror_ledger_receipt(kb_root, current, retained_archive.as_deref())?;
    } else if event.kind == "relocate" {
        mirror_ledger_receipt(kb_root, current, retained_archive.as_deref())?;
    }
    if event.kind == "supersede" {
        let old_id = event
            .supersedes
            .as_deref()
            .ok_or_else(|| "supersede event has no superseded receipt".to_string())?;
        let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
        journal
            .append(
                "node.upsert",
                &serde_json::json!({
                    "id": format!("artifact-revision:{}", old_id),
                    "status": "superseded",
                    "superseded_by": format!("artifact-revision:{}", current.id),
                    "provenance": event.provenance,
                }),
            )
            .map_err(|e| format!("ledger event synced but superseded node update failed: {e}"))?;
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": format!("artifact-revision:{}", current.id),
                    "to": format!("artifact-revision:{}", old_id),
                    "type": "SUPERSEDES",
                    "provenance": event.provenance,
                }),
            )
            .map_err(|e| format!("ledger event synced but supersession link failed: {e}"))?;
    }
    Ok(())
}

fn cmd_artifact_ledger(kb_root: &std::path::Path, format: &str, op: &LedgerOp) -> i32 {
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

fn cmd_cloud_status(format: &str, remote: &str) -> i32 {
    let bin = match resolve_rclone_bin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let r = innen_core::cloud::Rclone { bin };
    match r.status(remote) {
        Ok(output) => {
            if is_human(format) {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&CloudStatusJson {
                        remote: remote.to_string(),
                        output,
                    })
                    .expect("cloud status serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_cloud_doctor(format: &str) -> i32 {
    let bin = match resolve_rclone_bin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let r = innen_core::cloud::Rclone { bin };
    match r.doctor() {
        Ok(output) => {
            if is_human(format) {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&CloudDoctorJson { output })
                        .expect("cloud doctor serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_harvest(root: &std::path::Path, format: &str, _args: &HarvestArgs) -> i32 {
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

fn cmd_ingest(root: &std::path::Path, format: &str) -> i32 {
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

fn cmd_completions(shell: &str) -> i32 {
    let clap_shell = match shell {
        "bash" => clap_complete::Shell::Bash,
        "zsh" => clap_complete::Shell::Zsh,
        "fish" => clap_complete::Shell::Fish,
        "powershell" => clap_complete::Shell::PowerShell,
        _ => {
            eprintln!("error: unknown shell: {shell}");
            return 1;
        }
    };
    let mut cmd = Cli::command();
    clap_complete::generate(clap_shell, &mut cmd, "innen", &mut std::io::stdout());
    0
}

fn cmd_conversation(format: &str, args: &ConversationArgs) -> i32 {
    if let Some(path) = args
        .decode_packet
        .as_ref()
        .or(args.validate_packet.as_ref())
        .or(args.encode_json.as_ref())
    {
        let result = (|| -> Result<serde_json::Value, String> {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
                return Err("packet file exceeds 16 MiB admission bound".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if args.encode_json.is_some() {
                if !value["records"].is_array() || value.get("encoding").is_some() {
                    return Err(
                        "encode-json requires an ordinary conversation page with records".into(),
                    );
                }
                innen_core::conversation::grammar::encode(&value, &value)
            } else {
                let decoded = innen_core::conversation::grammar::decode(&value)?;
                Ok(if args.validate_packet.is_some() {
                    value
                } else {
                    decoded
                })
            }
        })();
        return match result {
            Ok(value) => {
                println!(
                    "{}",
                    if is_human(format) {
                        serde_json::to_string_pretty(&value).unwrap()
                    } else {
                        value.to_string()
                    }
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    let Some(uuid) = args.uuid.as_deref() else {
        eprintln!("error: missing conversation ID");
        return 1;
    };
    if args.codec == "conversation" && (args.attachment_refs || args.attachment.is_some()) {
        eprintln!("error: conversation grammar requires complete page data, not attachment externalization");
        return 1;
    }
    let result = if !args.lines.is_empty() {
        innen_core::conversation::read_lines(
            args.source_root.as_deref(),
            &args.source,
            uuid,
            &args.lines,
        )
    } else {
        innen_core::conversation::read(
            args.source_root.as_deref(),
            &args.source,
            uuid,
            if args.attachment.is_some() {
                "events"
            } else {
                &args.view
            },
            args.offset,
            if args.attachment.is_some() {
                1
            } else {
                usize::from(args.limit)
            },
        )
    };
    match result {
        Ok(page) => {
            let original = serde_json::to_value(&page).expect("page serializes");
            match innen_core::conversation::format_page(
                page,
                args.compact,
                args.deltas,
                args.attachment_refs,
                args.attachment.as_deref(),
                args.expect_sha256.as_deref(),
                args.offset,
            ) {
                Ok(formatted) => {
                    let formatted = if args.codec == "conversation" {
                        match innen_core::conversation::grammar::encode(&original, &formatted) {
                            Ok(value) => value,
                            Err(e) => {
                                eprintln!("error: {e}");
                                return 1;
                            }
                        }
                    } else {
                        formatted
                    };
                    let output = if is_human(format) {
                        serde_json::to_string_pretty(&formatted)
                    } else {
                        serde_json::to_string(&formatted)
                    };
                    println!("{}", output.expect("formatted page serializes"));
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn cmd_resume(format: &str, args: &ResumeArgs) -> i32 {
    let (uuid, source_override) = match &args.uuid {
        Some(id) => (id.clone(), None),
        None => {
            let project_dir = match &args.project {
                Some(p) => p.clone(),
                None => match std::env::current_dir() {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("error: failed to determine current working directory: {e}");
                        return 1;
                    }
                },
            };
            match innen_core::conversation::resume::resolve_resume_target(
                &project_dir,
                &args.source,
                args.source_root.as_deref(),
            ) {
                Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                    (c.id, Some(c.source.as_str().to_string()))
                }
                Ok(innen_core::conversation::resume::ResumeTarget::Ambiguous {
                    project,
                    candidates,
                }) => {
                    let report = serde_json::json!({
                        "status": "ambiguous",
                        "project": project,
                        "count": candidates.len(),
                        "candidates": candidates,
                        "message": "multiple candidate sessions found for project; specify session ID with: innen resume <session-id>"
                    });
                    if is_human(format) {
                        eprintln!(
                            "Multiple candidate sessions found for project {}:",
                            project.display()
                        );
                        for c in &candidates {
                            eprintln!(
                                "  - {} ({}, modified: {})",
                                c.id,
                                c.source.as_str(),
                                c.modified.as_deref().unwrap_or("unknown")
                            );
                        }
                        eprintln!("Specify session ID with: innen resume <session-id>");
                    } else {
                        println!("{}", serde_json::to_string(&report).unwrap());
                    }
                    return 1;
                }
                Ok(innen_core::conversation::resume::ResumeTarget::NotFound { project }) => {
                    let report = serde_json::json!({
                        "status": "not_found",
                        "project": project,
                        "message": format!(
                            "no candidate sessions found for project {}; specify session ID with: innen resume <session-id>",
                            project.display()
                        )
                    });
                    if is_human(format) {
                        eprintln!(
                            "No candidate sessions found for project {}.",
                            project.display()
                        );
                        eprintln!("Specify session ID with: innen resume <session-id>");
                    } else {
                        println!("{}", serde_json::to_string(&report).unwrap());
                    }
                    return 1;
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            }
        }
    };

    let source = source_override.as_deref().unwrap_or(&args.source);
    let view = args.view.as_deref().unwrap_or("brief");
    let offset = args.offset.unwrap_or(0);
    let limit = usize::from(args.limit.unwrap_or(20));
    let mut brief_fallback_warning = None;
    if view == "brief"
        && args.lines.is_empty()
        && args.attachment.is_none()
        && args.offset.is_none()
        && args.limit.is_none()
    {
        match innen_core::conversation::brief(args.source_root.as_deref(), source, &uuid) {
            Ok(mut brief) => {
                if !brief.supported {
                    if args.view.is_some() {
                        eprintln!("error: --view brief is unavailable for this source; use --view context or --view events");
                        return 1;
                    }
                    // The default remains useful for non-Codex adapters: use
                    // the established bounded context reader and state why.
                    brief_fallback_warning = Some("brief unavailable for this source adapter; returned bounded context projection".to_string());
                } else {
                    let checkpoint_project = args
                        .project
                        .clone()
                        .or_else(|| brief.project.clone().map(std::path::PathBuf::from));
                    if let Some(project) = checkpoint_project.as_deref() {
                        match innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                            project, &uuid,
                        ) {
                            Ok(checkpoint) => brief.checkpoint = checkpoint,
                            Err(error) => {
                                eprintln!("error: {error}");
                                return 1;
                            }
                        }
                    }
                    let value = serde_json::to_value(&brief).expect("resume brief serializes");
                    if is_human(format) {
                        println!("{}", serde_json::to_string_pretty(&value).unwrap());
                    } else {
                        println!("{}", serde_json::to_string(&value).unwrap());
                    }
                    return 0;
                }
            }
            Err(error) => {
                eprintln!("error: {error}");
                return 1;
            }
        }
    }
    let compact = !args.no_compact;
    let deltas = !args.no_deltas;

    let result = if !args.lines.is_empty() {
        innen_core::conversation::read_lines(
            args.source_root.as_deref(),
            source,
            &uuid,
            &args.lines,
        )
    } else {
        innen_core::conversation::read(
            args.source_root.as_deref(),
            source,
            &uuid,
            if args.attachment.is_some() {
                "events"
            } else if view == "brief" {
                "context"
            } else {
                view
            },
            offset,
            if args.attachment.is_some() { 1 } else { limit },
        )
    };

    match result {
        Ok(mut page) => {
            if let Some(warning) = brief_fallback_warning {
                page.warnings.push(warning);
            }
            match innen_core::conversation::format_page(
                page,
                compact,
                deltas,
                args.attachment_refs,
                args.attachment.as_deref(),
                args.expect_sha256.as_deref(),
                offset,
            ) {
                Ok(formatted) => {
                    let output = if is_human(format) {
                        serde_json::to_string_pretty(&formatted)
                    } else {
                        serde_json::to_string(&formatted)
                    };
                    println!("{}", output.expect("formatted page serializes"));
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn cmd_checkpoint(format: &str, args: &CheckpointArgs) -> i32 {
    let project_dir = match &args.project {
        Some(p) => p.clone(),
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: failed to determine current working directory: {e}");
                return 1;
            }
        },
    };

    match &args.op {
        Some(CheckpointOp::Record(rec)) => {
            let checkpoint = if let Some(ref file_arg) = rec.file {
                let content = if file_arg == "-" {
                    let mut s = String::new();
                    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s) {
                        eprintln!("error: failed to read checkpoint JSON from stdin: {e}");
                        return 1;
                    }
                    s
                } else {
                    match std::fs::read_to_string(file_arg) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("error: failed to read checkpoint file {file_arg}: {e}");
                            return 1;
                        }
                    }
                };
                match serde_json::from_str::<innen_core::conversation::Checkpoint>(&content) {
                    Ok(cp) => cp,
                    Err(e) => {
                        eprintln!("error: invalid checkpoint JSON: {e}");
                        return 1;
                    }
                }
            } else {
                let session = match &rec.session {
                    Some(s) => s.clone(),
                    None => {
                        match innen_core::conversation::resume::resolve_resume_target(
                            &project_dir,
                            &rec.source,
                            None,
                        ) {
                            Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                                c.id
                            }
                            Ok(innen_core::conversation::resume::ResumeTarget::Ambiguous {
                                ..
                            }) => {
                                eprintln!("error: multiple candidate sessions found; specify --session <id>");
                                return 1;
                            }
                            _ => {
                                eprintln!("error: --session <id> is required when no unambiguous session exists");
                                return 1;
                            }
                        }
                    }
                };

                let existing = innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                    &project_dir,
                    &session,
                )
                .ok()
                .flatten();

                let status = match innen_core::conversation::CheckpointStatus::parse(&rec.status) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 1;
                    }
                };

                let objective = if let Some(ref obj) = rec.objective {
                    obj.clone()
                } else if let Some(ref ex) = existing {
                    ex.objective.clone()
                } else {
                    eprintln!("error: --objective is required for a new checkpoint");
                    return 1;
                };

                let source = if rec.source != "auto" {
                    match innen_core::conversation::Source::parse(&rec.source) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 1;
                        }
                    }
                } else if let Some(ref ex) = existing {
                    ex.source
                } else {
                    innen_core::conversation::Source::Codex
                };

                let completed_work = if !rec.completed.is_empty() {
                    rec.completed.clone()
                } else if let Some(ref ex) = existing {
                    ex.completed_work.clone()
                } else {
                    Vec::new()
                };

                let current_evidence = if !rec.evidence.is_empty() {
                    rec.evidence.clone()
                } else if let Some(ref ex) = existing {
                    ex.current_evidence.clone()
                } else {
                    Vec::new()
                };

                let blockers = if !rec.blocker.is_empty() {
                    rec.blocker.clone()
                } else if let Some(ref ex) = existing {
                    ex.blockers.clone()
                } else {
                    Vec::new()
                };

                let next_action = rec.next_action.clone().unwrap_or_else(|| {
                    existing
                        .as_ref()
                        .map(|e| e.next_action.clone())
                        .unwrap_or_default()
                });

                let verification = rec.verification.clone().unwrap_or_else(|| {
                    existing
                        .as_ref()
                        .map(|e| e.verification.clone())
                        .unwrap_or_default()
                });

                innen_core::conversation::checkpoint::new_checkpoint(
                    session,
                    source,
                    project_dir.clone(),
                    status,
                    objective,
                    completed_work,
                    current_evidence,
                    blockers,
                    next_action,
                    verification,
                )
            };

            if let Err(e) =
                innen_core::conversation::checkpoint::record_checkpoint(&project_dir, &checkpoint)
            {
                eprintln!("error: {e}");
                return 1;
            }

            if is_human(format) {
                println!(
                    "Recorded checkpoint for session {} ({})",
                    checkpoint.session,
                    checkpoint.status.as_str()
                );
                println!("  Objective:   {}", checkpoint.objective);
                println!("  Status:      {}", checkpoint.status.as_str());
                println!("  Next action: {}", checkpoint.next_action);
            } else {
                let out = serde_json::json!({
                    "status": "recorded",
                    "checkpoint": checkpoint
                });
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            0
        }
        Some(CheckpointOp::Show(show_args)) => {
            let session = match &show_args.session {
                Some(s) => s.clone(),
                None => match &args.session {
                    Some(s) => s.clone(),
                    None => {
                        match innen_core::conversation::resume::resolve_resume_target(
                            &project_dir,
                            "auto",
                            None,
                        ) {
                            Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                                c.id
                            }
                            _ => {
                                eprintln!("error: specify session ID with: innen checkpoint show <session-id>");
                                return 1;
                            }
                        }
                    }
                },
            };

            match innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                &project_dir,
                &session,
            ) {
                Ok(Some(cp)) => {
                    if is_human(format) {
                        println!("Session:      {} ({})", cp.session, cp.source.as_str());
                        println!("Status:       {}", cp.status.as_str());
                        println!("Objective:    {}", cp.objective);
                        println!("Updated:      {}", cp.updated_at);
                        println!("Next action:  {}", cp.next_action);
                        println!("Verification: {}", cp.verification);
                        if !cp.completed_work.is_empty() {
                            println!("Completed work:");
                            for item in &cp.completed_work {
                                println!("  - {item}");
                            }
                        }
                        if !cp.current_evidence.is_empty() {
                            println!("Current evidence:");
                            for item in &cp.current_evidence {
                                println!("  - {item}");
                            }
                        }
                        if !cp.blockers.is_empty() {
                            println!("Blockers:");
                            for item in &cp.blockers {
                                println!("  - {item}");
                            }
                        }
                    } else {
                        println!("{}", serde_json::to_string(&cp).unwrap());
                    }
                    0
                }
                Ok(None) => {
                    let out = serde_json::json!({
                        "status": "not_found",
                        "session": session,
                        "message": format!("no checkpoint found for session {session}")
                    });
                    if is_human(format) {
                        eprintln!("No checkpoint found for session {session}.");
                    } else {
                        println!("{}", serde_json::to_string(&out).unwrap());
                    }
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        Some(CheckpointOp::List(_)) | None => {
            if let Some(ref session) = args.session {
                return cmd_checkpoint(
                    format,
                    &CheckpointArgs {
                        op: Some(CheckpointOp::Show(CheckpointShowArgs {
                            session: Some(session.clone()),
                            project: Some(project_dir),
                        })),
                        session: None,
                        project: None,
                    },
                );
            }
            match innen_core::conversation::checkpoint::latest_checkpoints(&project_dir) {
                Ok(list) => {
                    if is_human(format) {
                        if list.is_empty() {
                            println!(
                                "No checkpoints found for project {}.",
                                project_dir.display()
                            );
                        } else {
                            println!("Checkpoints for project {}:", project_dir.display());
                            for cp in &list {
                                println!(
                                    "  - {} [{}] {} (updated: {})",
                                    cp.session,
                                    cp.status.as_str(),
                                    cp.objective,
                                    cp.updated_at
                                );
                            }
                        }
                    } else {
                        let out = serde_json::json!({
                            "status": "ok",
                            "project": project_dir,
                            "count": list.len(),
                            "checkpoints": list
                        });
                        println!("{}", serde_json::to_string(&out).unwrap());
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
}

fn cmd_unfinished(format: &str, args: &UnfinishedArgs) -> i32 {
    let source_filter = if args.source == "auto" {
        None
    } else {
        match innen_core::conversation::Source::parse(&args.source) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };

    if args.all_projects {
        match innen_core::conversation::unfinished::find_all_unfinished_candidates(
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        ) {
            Ok(candidates) => {
                if is_human(format) {
                    if candidates.is_empty() {
                        println!("No unfinished candidate sessions found across all projects.");
                    } else {
                        println!("Unfinished candidate sessions across all projects:");
                        for c in &candidates {
                            println!(
                                "  - {} ({}, project: {}, modified: {}, confidence: {:?})",
                                c.id,
                                c.source.as_str(),
                                c.project,
                                c.modified.as_deref().unwrap_or("unknown"),
                                c.confidence
                            );
                            for r in &c.reasons {
                                println!("      * {}: {}", r.code, r.detail);
                            }
                            if let Some(ref cp) = c.checkpoint {
                                println!(
                                    "      Objective: {} [{}]",
                                    cp.objective,
                                    cp.status.as_str()
                                );
                            }
                            println!("      Resume with: innen resume {}", c.id);
                        }
                    }
                } else {
                    let out = serde_json::json!({
                        "status": "ok",
                        "all_projects": true,
                        "since": args.since,
                        "count": candidates.len(),
                        "candidates": candidates
                    });
                    println!("{}", serde_json::to_string(&out).unwrap());
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    } else {
        let project_dir = match &args.project {
            Some(p) => p.clone(),
            None => match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: failed to determine current working directory: {e}");
                    return 1;
                }
            },
        };

        match innen_core::conversation::unfinished::find_unfinished_candidates(
            &project_dir,
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        ) {
            Ok(candidates) => {
                if is_human(format) {
                    if candidates.is_empty() {
                        println!(
                            "No unfinished candidate sessions found for project {}.",
                            project_dir.display()
                        );
                    } else {
                        println!(
                            "Unfinished candidate sessions for project {}:",
                            project_dir.display()
                        );
                        for c in &candidates {
                            println!(
                                "  - {} ({}, modified: {}, confidence: {:?})",
                                c.id,
                                c.source.as_str(),
                                c.modified.as_deref().unwrap_or("unknown"),
                                c.confidence
                            );
                            for r in &c.reasons {
                                println!("      * {}: {}", r.code, r.detail);
                            }
                            if let Some(ref cp) = c.checkpoint {
                                println!(
                                    "      Objective: {} [{}]",
                                    cp.objective,
                                    cp.status.as_str()
                                );
                            }
                            println!("      Resume with: innen resume {}", c.id);
                        }
                    }
                } else {
                    let out = serde_json::json!({
                        "status": "ok",
                        "project": project_dir,
                        "since": args.since,
                        "count": candidates.len(),
                        "candidates": candidates
                    });
                    println!("{}", serde_json::to_string(&out).unwrap());
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

fn cmd_pickup(format: &str, args: &PickupArgs) -> i32 {
    let source_filter = if args.source == "auto" {
        None
    } else {
        match innen_core::conversation::Source::parse(&args.source) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };

    let target_result = if args.all_projects {
        innen_core::conversation::pickup::resolve_all_projects_pickup(
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        )
    } else {
        let project_dir = match &args.project {
            Some(p) => p.clone(),
            None => match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: failed to determine current working directory: {e}");
                    return 1;
                }
            },
        };
        innen_core::conversation::pickup::resolve_pickup(
            &project_dir,
            args.uuid.as_deref(),
            source_filter,
            args.source_root.as_deref(),
            args.since.as_deref(),
        )
    };

    match target_result {
        Ok(innen_core::conversation::pickup::PickupTarget::PickedUp(session)) => {
            if is_human(format) {
                println!(
                    "Picked up session: {} ({})",
                    session.session_id,
                    session.source.as_str()
                );
                println!("Project:        {}", session.project.display());
                if let Some(ref cp) = session.checkpoint {
                    println!("Status:         {}", cp.status.as_str());
                    println!("Objective:      {}", cp.objective);
                }
                println!("Confidence:     {:?}", session.evidence_pointers.confidence);
                println!("Reasons:");
                for r in &session.evidence_pointers.reasons {
                    println!("  - {}: {}", r.code, r.detail);
                }
                println!("Next action:    {}", session.next_action);
                println!("Verification:   {}", session.verification);
                println!("Resume command: {}", session.resume_command);
            } else {
                let out = serde_json::json!({
                    "status": "picked_up",
                    "session_id": session.session_id,
                    "source": session.source,
                    "project": session.project,
                    "checkpoint": session.checkpoint,
                    "evidence_pointers": session.evidence_pointers,
                    "next_action": session.next_action,
                    "verification": session.verification,
                    "resume_command": session.resume_command
                });
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            0
        }
        Ok(innen_core::conversation::pickup::PickupTarget::Ambiguous {
            project,
            count,
            candidates,
            message,
        }) => {
            let out = serde_json::json!({
                "status": "ambiguous",
                "project": project,
                "count": count,
                "candidates": candidates,
                "message": message
            });
            if is_human(format) {
                if project.to_string_lossy() == "all-projects" {
                    eprintln!("Multiple candidate sessions found across projects:");
                    for c in &candidates {
                        eprintln!(
                            "  - {} ({}, project: {}, modified: {}, confidence: {:?})",
                            c.id,
                            c.source.as_str(),
                            c.project,
                            c.modified.as_deref().unwrap_or("unknown"),
                            c.confidence
                        );
                    }
                } else {
                    eprintln!(
                        "Multiple candidate sessions found for project {}:",
                        project.display()
                    );
                    for c in &candidates {
                        eprintln!(
                            "  - {} ({}, modified: {}, confidence: {:?})",
                            c.id,
                            c.source.as_str(),
                            c.modified.as_deref().unwrap_or("unknown"),
                            c.confidence
                        );
                    }
                }
                eprintln!("Pick up explicit session with: innen pickup <session-id>");
            } else {
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            1
        }
        Ok(innen_core::conversation::pickup::PickupTarget::NotFound { project, message }) => {
            let out = serde_json::json!({
                "status": "not_found",
                "project": project,
                "count": 0,
                "candidates": [],
                "message": message
            });
            if is_human(format) {
                if project.to_string_lossy() == "all-projects" {
                    eprintln!("No candidate sessions found across any projects.");
                } else {
                    eprintln!(
                        "No candidate sessions found for project {}.",
                        project.display()
                    );
                }
                eprintln!("{message}");
            } else {
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run() -> i32 {
    let cli = Cli::parse();
    macro_rules! resolve_or_exit {
        ($cli_root:expr) => {
            match resolve_root($cli_root) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            }
        };
    }
    match &cli.command {
        Commands::Conversation(args) => cmd_conversation(&cli.format, args),
        Commands::Resume(args) => cmd_resume(&cli.format, args),
        Commands::Checkpoint(args) => cmd_checkpoint(&cli.format, args),
        Commands::Unfinished(args) => cmd_unfinished(&cli.format, args),
        Commands::Pickup(args) => cmd_pickup(&cli.format, args),
        Commands::Query(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_query(&root, &cli.format, args)
        }
        Commands::Graph { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                GraphOp::Node(a) => cmd_graph_node(&root, &cli.format, a),
                GraphOp::Relate(a) => cmd_graph_relate(&root, &cli.format, a),
                GraphOp::Retract(a) => cmd_graph_retract(&root, &cli.format, a),
            }
        }
        Commands::Index { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                IndexOp::Rebuild => cmd_index_rebuild(&root, &cli.format),
            }
        }
        Commands::Doctor => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_doctor(&root, &cli.format)
        }
        Commands::Lint => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_lint(&root, &cli.format)
        }
        Commands::Config { op } => match op {
            ConfigOp::Get { key, global } => {
                if *global {
                    cmd_config_get_global(&cli.format, key)
                } else {
                    let root = resolve_or_exit!(cli.root.clone());
                    cmd_config_get(&root, &cli.format, key)
                }
            }
            ConfigOp::Set { key, value, global } => {
                if *global {
                    cmd_config_set_global(&cli.format, key, value)
                } else {
                    let root = resolve_or_exit!(cli.root.clone());
                    cmd_config_set(&root, &cli.format, key, value)
                }
            }
        },
        Commands::Completions { shell } => cmd_completions(shell),
        Commands::Guide => cmd_guide(&cli.format),
        Commands::Search(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_search(&root, &cli.format, args)
        }
        Commands::Status => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_status(&root, &cli.format)
        }
        Commands::Timeline(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_timeline(&root, &cli.format, args)
        }
        Commands::Project(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            if args.view == "full" {
                match &args.id {
                    Some(id) => cmd_project(&root, &cli.format, id),
                    None => {
                        eprintln!("--view full requires a project id");
                        1
                    }
                }
            } else {
                let options = innen_core::project_brief::TaskOptions {
                    project: args.id.as_deref(),
                    all: args.view == "evidence",
                    detail: args.view == "evidence",
                    offset: args.offset,
                    limit: usize::from(args.limit),
                };
                match innen_core::project_brief::list(&root, &options) {
                    Ok(out) => {
                        if is_human(&cli.format) {
                            print!("{}", innen_core::project_brief::render(&out));
                        } else {
                            println!("{}", out);
                        }
                        0
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                }
            }
        }
        Commands::Profile => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_profile(&root, &cli.format)
        }
        Commands::Artifact { op } => {
            let root = resolve_or_exit!(cli.root.clone());
            match op {
                ArtifactOp::Add(a) => cmd_artifact_add(&root, &cli.format, a),
                ArtifactOp::AddTree(a) => cmd_artifact_add_tree(&root, &cli.format, a),
                ArtifactOp::Ledger { op } => cmd_artifact_ledger(&root, &cli.format, op),
            }
        }
        Commands::Cloud { op } => match op {
            CloudOp::Status(a) => cmd_cloud_status(&cli.format, &a.remote),
            CloudOp::Doctor => cmd_cloud_doctor(&cli.format),
        },
        Commands::Harvest(args) => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_harvest(&root, &cli.format, args)
        }
        Commands::Ingest => {
            let root = resolve_or_exit!(cli.root.clone());
            cmd_ingest(&root, &cli.format)
        }
    }
}

fn main() {
    std::process::exit(run());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_escape_pins_tabs_newlines_and_cjk_truncation() {
        // Embedded tabs/newlines must not break TSV rows.
        assert_eq!(escape_tsv_field("a\tb\nc\rd"), "a\\tb\\nc\\rd");
        // Long fields truncate at 200 chars (chars, not bytes — CJK safe) with ….
        let long = "字".repeat(250);
        let got = escape_tsv_field(&long);
        assert_eq!(got, format!("{}…", "字".repeat(TSV_FIELD_LIMIT)));
        assert_eq!(got.chars().count(), TSV_FIELD_LIMIT + 1);
        // Short CJK passes through unchanged.
        assert_eq!(escape_tsv_field("臺北"), "臺北");
    }
}

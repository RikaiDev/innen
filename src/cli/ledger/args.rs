use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub(in crate::cli) enum LedgerOp {
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
pub(in crate::cli) struct LedgerInitArgs {
    #[arg(long = "archive-root")]
    pub(super) archive_root: PathBuf,
    #[arg(long)]
    pub(super) project_id: String,
    #[arg(long)]
    pub(super) title: String,
    #[arg(long)]
    pub(super) downloads_root: Option<PathBuf>,
    #[arg(long, value_delimiter = ',')]
    pub(super) categories: Vec<String>,
}

#[derive(clap::Args)]
pub(in crate::cli) struct LedgerAddArgs {
    #[arg(long = "archive-root")]
    pub(super) archive_root: PathBuf,
    #[arg(long)]
    pub(super) file: PathBuf,
    #[arg(long)]
    pub(super) source_kind: String,
    #[arg(long)]
    pub(super) role: String,
    #[arg(long)]
    pub(super) stage: String,
    #[arg(long)]
    pub(super) owner_project: String,
    /// Archive project identity (A); owner_project may identify a separate
    /// project (B) for a reference receipt.
    #[arg(long)]
    pub(super) archive_project_id: Option<String>,
    #[arg(long)]
    pub(super) relation: String,
    #[arg(long)]
    pub(super) document_date: Option<String>,
    #[arg(long)]
    pub(super) authority: Option<String>,
    #[arg(long)]
    pub(super) evidence_status: Option<String>,
    #[arg(long)]
    pub(super) document_id: Option<String>,
    #[arg(long)]
    pub(super) approval_valid_from: Option<String>,
    #[arg(long)]
    pub(super) approval_valid_until: Option<String>,
    #[arg(long)]
    pub(super) provenance: String,
    #[arg(long)]
    pub(super) downloads_root: Option<PathBuf>,
}

#[derive(clap::Args)]
pub(in crate::cli) struct LedgerCheckArgs {
    #[arg(long = "archive-root")]
    pub(super) archive_root: PathBuf,
    #[arg(long)]
    pub(super) project_id: String,
    #[arg(long)]
    pub(super) title: String,
}

#[derive(clap::Args)]
pub(in crate::cli) struct LedgerEventArgs {
    #[arg(long = "archive-root")]
    pub(super) archive_root: PathBuf,
    #[arg(long)]
    pub(super) project_id: String,
    #[arg(long)]
    pub(super) title: String,
    #[arg(long)]
    pub(super) kind: String,
    #[arg(long)]
    pub(super) receipt_id: String,
    #[arg(long)]
    pub(super) superseded_receipt_id: Option<String>,
    #[arg(long)]
    pub(super) new_path: Option<PathBuf>,
    #[arg(long)]
    pub(super) expected_sha256: Option<String>,
    #[arg(long)]
    pub(super) event_date: Option<String>,
    #[arg(long)]
    pub(super) note: Option<String>,
    /// New owner project for a reclassification; omitted keeps the recorded owner.
    #[arg(long)]
    pub(super) new_owner_project: Option<String>,
    /// New relation (`belongs_to` or `reference`) for a reclassification.
    #[arg(long)]
    pub(super) new_relation: Option<String>,
    /// Evidence explaining a reclassification (required for that event kind).
    #[arg(long)]
    pub(super) provenance: Option<String>,
    /// Archive project identity to retain when mirroring a lifecycle event.
    #[arg(long)]
    pub(super) archive_project_id: Option<String>,
}

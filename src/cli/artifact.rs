use super::ledger::LedgerOp;
#[derive(clap::Subcommand)]
pub(super) enum ArtifactOp {
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
#[derive(clap::Args)]
pub(super) struct ArtifactAddArgs {
    /// Source file to store.
    #[arg(long)]
    pub(super) file: PathBuf,
    /// Optional project id for a BELONGS_TO edge.
    #[arg(long)]
    pub(super) project: Option<String>,
}

#[derive(clap::Args)]
pub(super) struct ArtifactAddTreeArgs {
    /// Directory to inventory without following symlinks.
    #[arg(long)]
    pub(super) directory: PathBuf,
    /// Manifest output path outside the inventoried directory.
    #[arg(long)]
    pub(super) manifest: PathBuf,
    /// Optional project id for a BELONGS_TO edge.
    #[arg(long)]
    pub(super) project: Option<String>,
}

#[derive(serde::Serialize)]
pub(super) struct ArtifactJson {
    pub(super) sha256: String,
    pub(super) path: String,
    pub(super) bytes: u64,
}

#[derive(serde::Serialize)]
pub(super) struct ArtifactTreeJson {
    pub(super) tree_sha256: String,
    pub(super) files: u64,
    pub(super) symlinks: u64,
    pub(super) source_bytes: u64,
    pub(super) manifest_sha256: String,
    pub(super) manifest_path: String,
    pub(super) stored_path: String,
    pub(super) metadata_only: bool,
}

use super::util::{escape_tsv_field, is_human};
use std::path::PathBuf;

// P2 `artifact add --file <path> [--project <id>]`.

pub(super) fn cmd_artifact_add(
    root: &std::path::Path,
    format: &str,
    args: &ArtifactAddArgs,
) -> i32 {
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

pub(super) fn cmd_artifact_add_tree(
    root: &std::path::Path,
    format: &str,
    args: &ArtifactAddTreeArgs,
) -> i32 {
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

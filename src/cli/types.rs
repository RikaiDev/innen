use super::commands::Commands;
use std::path::PathBuf;

#[derive(clap::Parser)]
#[command(
    name = "innen",
    version,
    about = "innen P1+P2 knowledge CLI",
    long_about = "innen P1+P2 knowledge CLI.\n\nKB root resolution: --root <dir> > INNEN_ROOT env (non-empty) > user-level persisted root (~/.config/innen/config.json).\n\nOperational boundaries:\n  harvest: imports conversation transcripts into knowledge graph entities\n  checkpoint: records progress snapshots in append-only storage outside Git\n  unfinished: discovers candidate unfinished conversations using structural evidence\n  resume / pickup: reconstructs working context and provides continuation instructions"
)]
pub(super) struct Cli {
    /// Output format (explicit only; no TTY sniffing). Applies to all commands.
    #[arg(long, global = true, default_value = "json", value_parser = ["json", "human"])]
    pub(super) format: String,
    /// KB root dir. Precedence: --root > INNEN_ROOT env > global config.
    #[arg(long, global = true)]
    pub(super) root: Option<PathBuf>,
    #[command(subcommand)]
    pub(super) command: Commands,
}

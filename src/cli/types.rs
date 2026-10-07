use super::commands::Commands;
use std::path::PathBuf;

/// What an agent or newcomer needs first: intent, not a 22-row command table.
/// The table stays below for lookups; this answers "what do I type".
const INTENT_INDEX: &str = "\
Pick the line that matches what you want; the full command list is below it.

  Find something recorded before   query <any words>            (also: query --q \"...\")
  Continue an unfinished session   unfinished  ->  pickup
  Read one known conversation      conversation <ses_...>
  See what a project still owes    project <project-id>
  Trace a clue back to its source  trace <any words>
  Check whether state is sound     doctor
  Not sure where to start          guide
";

const BOUNDARIES: &str = "\
KB root resolution: --root <dir> > INNEN_ROOT env (non-empty) > user-level persisted root (~/.config/innen/config.json).

Operational boundaries:
  harvest: imports conversation transcripts into knowledge graph entities
  checkpoint: records progress snapshots in append-only storage outside Git
  unfinished: discovers candidate unfinished conversations using structural evidence
  resume / pickup: reconstructs working context and provides continuation instructions
";

/// Long help keeps the intent index last so it is the last thing read before the
/// command table it routes into.
fn long_about() -> String {
    format!("innen P1+P2 knowledge CLI.\n\n{BOUNDARIES}\n{INTENT_INDEX}")
}

#[derive(clap::Parser)]
#[command(
    name = "innen",
    version,
    about = "innen P1+P2 knowledge CLI",
    before_help = INTENT_INDEX,
    long_about = long_about()
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

//! Agent stop hooks: `innen hook install|run|pending`.
//!
//! Split by responsibility: [`run`] captures worktree state into the harvest
//! inbox, [`pending`] lists what is waiting there, and [`install`] writes each
//! agent's hook wiring. Every runtime error degrades to stderr and exit 0 so a
//! hook never fails the calling agent; only `install`, which a human invokes,
//! returns nonzero on IO errors.

mod agents;
mod install;
mod pending;
mod plugin;
mod run;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub(super) struct HookArgs {
    #[command(subcommand)]
    pub(super) op: HookOp,
}

#[derive(clap::Subcommand)]
pub(super) enum HookOp {
    /// Write agent hook config files (idempotent merge).
    Install(HookInstallArgs),
    /// Snapshot worktree state into the harvest inbox (hook runtime).
    Run(HookRunArgs),
    /// List unprocessed pending snapshots (for SessionStart context).
    Pending,
}

#[derive(clap::Args)]
pub(super) struct HookInstallArgs {
    /// Target coding agent.
    #[arg(long, value_parser = ["claude-code", "codex", "opencode", "antigravity", "grok"])]
    pub(super) agent: String,
    /// Config scope (default: every machine you own gets it).
    #[arg(long, default_value = "user", value_parser = ["user", "project"])]
    pub(super) scope: String,
    /// KB root to bake into hook commands (default: resolved global root).
    #[arg(long)]
    pub(super) kb_root: Option<PathBuf>,
    /// Project dir for project scope (default: current dir).
    #[arg(long)]
    pub(super) cwd: Option<PathBuf>,
}

#[derive(clap::Args)]
pub(super) struct HookRunArgs {
    /// Hook event (per-agent wiring passes the matching one).
    #[arg(long, value_parser = ["session-end", "stop", "session-idle", "compact", "pre-compact"])]
    pub(super) event: String,
    /// Session id (flag overrides hook stdin).
    #[arg(long)]
    pub(super) session_id: Option<String>,
    /// Transcript path (flag overrides hook stdin).
    #[arg(long)]
    pub(super) transcript: Option<String>,
    /// Work dir to snapshot (flag > hook stdin > current dir).
    ///
    /// Antigravity sends `workspacePaths` (an array) and no `cwd`, and runs the
    /// hook with its cwd set to the directory holding `hooks.json`. Resolving
    /// `workspacePaths[0]` is what lets an Antigravity turn name the real
    /// workspace instead of that non-repository directory.
    #[arg(long)]
    pub(super) cwd: Option<PathBuf>,
    /// KB root owning the harvest inbox (default: resolved global root).
    #[arg(long)]
    pub(super) kb_root: Option<PathBuf>,
}

pub(super) fn cmd_hook(root: &Path, format: &str, args: &HookArgs) -> i32 {
    match &args.op {
        HookOp::Install(a) => install::cmd_hook_install(root, format, a),
        HookOp::Run(a) => run::cmd_hook_run(root, format, a),
        HookOp::Pending => pending::cmd_hook_pending(root, format),
    }
}

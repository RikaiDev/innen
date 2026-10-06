//! Snapshot capture: git worktree state -> `pending-*.md` in the harvest inbox.
//!
//! Capture is mechanical; judging and write-back stay with agents. `hook run`
//! never fails the calling agent: every runtime error degrades to stderr and
//! exit 0.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use super::HookRunArgs;
// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// Max dirty-file sample lines kept in a snapshot.
pub(super) const SAMPLE_LIMIT: usize = 20;
/// Max chars per sample line (chars, not bytes — CJK safe).
pub(super) const SAMPLE_LINE_LIMIT: usize = 300;

pub(super) struct Snapshot {
    pub(super) repo: String,
    pub(super) branch: String,
    pub(super) dirty_count: usize,
    pub(super) digest: String,
    pub(super) sample: Vec<String>,
}

pub(super) fn first_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let obj = v.as_object()?;
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
        // A list-valued key stands in for a scalar one: Antigravity sends
        // `workspacePaths: ["/path"]` where other agents send `cwd: "/path"`.
        if let Some(first) = obj
            .get(*k)
            .and_then(|x| x.as_array())
            .and_then(|a| a.iter().find_map(|v| v.as_str()))
        {
            if !first.trim().is_empty() {
                return Some(first.to_string());
            }
        }
    }
    None
}

pub(super) fn read_hook_stdin() -> serde_json::Value {
    if std::io::stdin().is_terminal() {
        return serde_json::Value::Null;
    }
    let mut buf = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).is_err() {
        return serde_json::Value::Null;
    }
    serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
}

pub(super) fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Root of the Git worktree containing `cwd`, or `None` when `cwd` is not
/// inside a repository.
pub(super) fn worktree_root(cwd: &Path) -> Option<PathBuf> {
    let top = git_output(cwd, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_string();
    if top.is_empty() {
        return None;
    }
    Some(PathBuf::from(top))
}

/// Snapshot the worktree containing `cwd`.
///
/// Returns `None` for a directory that is not inside a Git worktree. Such a
/// directory has no repository, no branch and no diff, so a snapshot of it
/// could never carry anything to harvest; recording `repo: <cwd>` with
/// `branch: nongit` asserted a repository and a branch that do not exist and
/// let an agent fire from any unrelated directory. Skipping here instead of in
/// the generated wiring also means `hook install` cannot regress it.
///
/// `branch` still reads `nongit` for a worktree with an unborn HEAD; after
/// this gate that value can no longer mean "not a repository".
pub(super) fn snapshot_repo(cwd: &Path) -> Option<Snapshot> {
    let top = worktree_root(cwd)?;
    let status = git_output(&top, &["status", "--porcelain=v1"]).unwrap_or_default();
    let branch = git_output(&top, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "nongit".to_string());
    let lines: Vec<&str> = status.lines().filter(|l| !l.trim().is_empty()).collect();
    let sample = lines
        .iter()
        .take(SAMPLE_LIMIT)
        .map(|l| {
            let mut s: String = l.chars().take(SAMPLE_LINE_LIMIT).collect();
            if l.chars().count() > SAMPLE_LINE_LIMIT {
                s.push('…');
            }
            s
        })
        .collect::<Vec<_>>();
    let canonical = format!("{}\n{branch}\n{status}", top.display());
    let digest = innen_core::ids::sha256_hex(canonical.as_bytes())[..12].to_string();
    Some(Snapshot {
        repo: top.to_string_lossy().into_owned(),
        branch,
        dirty_count: lines.len(),
        digest,
        sample,
    })
}

pub(super) fn pending_name(epoch: u64, digest: &str) -> String {
    format!("pending-{epoch}-{digest}.md")
}

/// Receipt identity for one worktree state, identity, and event.
///
/// The identity-less case deliberately does not fall back to the clock. An
/// agent that supplies neither a session id nor a transcript path (Antigravity
/// Stop, or `session.idle` without a session id) would otherwise mint a fresh
/// digest on every firing and grow the harvest inbox without limit. Distinct
/// worktree states still produce distinct receipts, which is what a harvester
/// needs; repeated firings over an unchanged state deduplicate to one.
pub(super) fn receipt_digest(
    worktree_digest: &str,
    session: &str,
    transcript: &str,
    event: &str,
) -> String {
    let identity = if session != "-" {
        session.to_string()
    } else if transcript != "-" {
        transcript.to_string()
    } else {
        "no-identity".to_string()
    };
    let key = format!("{worktree_digest}|{identity}|{event}");
    innen_core::ids::sha256_hex(key.as_bytes())[..12].to_string()
}

pub(super) fn inbox_has_digest(inbox: &Path, digest: &str) -> bool {
    let suffix = format!("-{digest}.md");
    std::fs::read_dir(inbox)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.file_name().to_string_lossy().starts_with("pending-")
                    && e.file_name().to_string_lossy().ends_with(suffix.as_str())
            })
        })
        .unwrap_or(false)
}

pub(super) fn render_snapshot_md(
    event: &str,
    session: &str,
    transcript: &str,
    epoch: u64,
    snap: &Snapshot,
) -> String {
    let mut out = String::new();
    out.push_str("# Stop-hook snapshot (pending)\n\n");
    out.push_str(&format!("- event: {event}\n"));
    out.push_str(&format!("- session: {session}\n"));
    out.push_str(&format!("- transcript: {transcript}\n"));
    out.push_str(&format!("- repo: {}\n", snap.repo));
    out.push_str(&format!("- branch: {}\n", snap.branch));
    out.push_str(&format!("- dirty: {}\n", snap.dirty_count));
    out.push_str(&format!("- digest: {}\n", snap.digest));
    out.push_str(&format!("- created_utc: {epoch}\n\n"));
    out.push_str("## Changed files (sample)\n\n");
    if snap.sample.is_empty() {
        out.push_str("(clean)\n");
    } else {
        for l in &snap.sample {
            out.push_str(&format!("- {l}\n"));
        }
        if snap.dirty_count > snap.sample.len() {
            out.push_str(&format!(
                "- … ({} more)\n",
                snap.dirty_count - snap.sample.len()
            ));
        }
    }
    out.push_str(
        "\n> Agent: verify claims against the repo, record `innen graph` nodes and\n\
         > wiki/log updates, then delete this file after ingesting.\n",
    );
    if session != "-"
        && session.len() <= 128
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        out.push_str(&format!(
            "\n> For a Codex session, check final-answer document links with `innen harvest --check --source codex --session {session}`; verify and register deliverables explicitly.\n"
        ));
    }
    out
}

pub(super) fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(super) fn cmd_hook_run(root: &Path, format: &str, args: &HookRunArgs) -> i32 {
    // Fail-open: hook runtime must never break the calling agent.
    let outcome = hook_run_inner(root, args);
    let human = crate::cli::util::is_human(format);
    let is_decision = matches!(args.event.as_str(), "stop" | "compact" | "pre-compact");
    match outcome {
        Ok(Receipt::Wrote { name, .. }) => {
            if is_decision {
                println!("{{\"decision\":\"allow\"}}");
            } else if human {
                println!("wrote\t{name}");
            } else {
                println!("{{\"wrote\":{name:?}}}");
            }
        }
        Ok(Receipt::Deduped { digest }) => {
            if is_decision {
                println!("{{\"decision\":\"allow\"}}");
            } else if human {
                println!("deduped\tno-change");
            } else {
                println!("{{\"deduped\":\"no-change\",\"digest\":{digest:?}}}");
            }
        }
        Ok(Receipt::OutsideRepository { dir }) => {
            if is_decision {
                println!("{{\"decision\":\"allow\"}}");
            } else if human {
                println!("skipped\tnot-a-repository\t{dir}");
            } else {
                println!("{{\"skipped\":\"not-a-repository\",\"dir\":{dir:?}}}");
            }
        }
        Err(e) => {
            eprintln!("innen hook run degraded: {e}");
            if is_decision {
                println!("{{\"decision\":\"allow\"}}");
            }
        }
    }
    0
}

pub(super) enum Receipt {
    Wrote {
        name: String,
    },
    /// This worktree state, identity, and event already has a receipt.
    Deduped {
        digest: String,
    },
    /// Not inside a Git worktree, so there is no repository state to record.
    OutsideRepository {
        dir: String,
    },
}

fn hook_run_inner(root: &Path, args: &HookRunArgs) -> Result<Receipt, String> {
    let kb = match &args.kb_root {
        Some(p) => p.clone(),
        None => root.to_path_buf(),
    };
    let stdin_json = read_hook_stdin();
    let cwd = match &args.cwd {
        Some(p) => p.clone(),
        None => first_str(
            &stdin_json,
            &[
                // Antigravity: no `cwd` at all, and its process cwd is the
                // hooks.json directory. `workspacePaths` is the real workspace.
                "workspacePaths",
                "workspace_paths",
                "cwd",
                "working_directory",
                "workspaceRoot",
                "workspace_root",
            ],
        )
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "no work dir".to_string())?,
    };
    let session = args
        .session_id
        .clone()
        .or_else(|| {
            first_str(
                &stdin_json,
                &[
                    "session_id",
                    "sessionId",
                    // Antigravity's identity field.
                    "conversationId",
                ],
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let transcript = args
        .transcript
        .clone()
        .or_else(|| {
            first_str(
                &stdin_json,
                &["transcript_path", "transcriptPath", "transcript"],
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let Some(mut snap) = snapshot_repo(&cwd) else {
        return Ok(Receipt::OutsideRepository {
            dir: cwd.to_string_lossy().into_owned(),
        });
    };
    // A clean Git tree can still have a delivered file outside the repository.
    // Keep one pending receipt per session/event instead of deduplicating all
    // such deliveries under the same worktree digest.
    let epoch = epoch_now();
    snap.digest = receipt_digest(&snap.digest, &session, &transcript, &args.event);
    let inbox = kb.join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).map_err(|e| format!("inbox mkdir: {e}"))?;
    if inbox_has_digest(&inbox, &snap.digest) {
        return Ok(Receipt::Deduped {
            digest: snap.digest,
        });
    }
    let name = pending_name(epoch, &snap.digest);
    let body = render_snapshot_md(&args.event, &session, &transcript, epoch, &snap);
    std::fs::write(inbox.join(&name), body).map_err(|e| format!("inbox write: {e}"))?;
    Ok(Receipt::Wrote { name })
}

//! Agent stop hooks: `innen hook install|run|pending`.
//!
//! Capture is mechanical (git snapshot → `pending-*.md` in the harvest
//! inbox); judging and write-back stay with agents. `hook run` never fails
//! the calling agent: all runtime errors degrade to stderr + exit 0.
//! Only `install` (human-invoked) returns nonzero on IO errors.

use std::io::IsTerminal as _;
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
    #[arg(long, value_parser = ["session-end", "stop", "session-idle"])]
    pub(super) event: String,
    /// Session id (flag overrides hook stdin).
    #[arg(long)]
    pub(super) session_id: Option<String>,
    /// Transcript path (flag overrides hook stdin).
    #[arg(long)]
    pub(super) transcript: Option<String>,
    /// Work dir to snapshot (flag > hook stdin > current dir).
    #[arg(long)]
    pub(super) cwd: Option<PathBuf>,
    /// KB root owning the harvest inbox (default: resolved global root).
    #[arg(long)]
    pub(super) kb_root: Option<PathBuf>,
}

pub(super) fn cmd_hook(root: &Path, format: &str, args: &HookArgs) -> i32 {
    match &args.op {
        HookOp::Install(a) => cmd_hook_install(root, format, a),
        HookOp::Run(a) => cmd_hook_run(root, format, a),
        HookOp::Pending => cmd_hook_pending(root, format),
    }
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// Max dirty-file sample lines kept in a snapshot.
const SAMPLE_LIMIT: usize = 20;
/// Max chars per sample line (chars, not bytes — CJK safe).
const SAMPLE_LINE_LIMIT: usize = 300;

struct Snapshot {
    repo: String,
    branch: String,
    dirty_count: usize,
    digest: String,
    sample: Vec<String>,
}

fn first_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let obj = v.as_object()?;
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn read_hook_stdin() -> serde_json::Value {
    if std::io::stdin().is_terminal() {
        return serde_json::Value::Null;
    }
    let mut buf = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).is_err() {
        return serde_json::Value::Null;
    }
    serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
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

fn snapshot_repo(cwd: &Path) -> Snapshot {
    let top = git_output(cwd, &["rev-parse", "--show-toplevel"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| cwd.to_string_lossy().into_owned());
    let branch = git_output(Path::new(&top), &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "nongit".to_string());
    let status = git_output(Path::new(&top), &["status", "--porcelain=v1"]).unwrap_or_default();
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
    let canonical = format!("{top}\n{branch}\n{status}");
    let digest = innen_core::ids::sha256_hex(canonical.as_bytes())[..12].to_string();
    Snapshot {
        repo: top,
        branch,
        dirty_count: lines.len(),
        digest,
        sample,
    }
}

fn pending_name(epoch: u64, digest: &str) -> String {
    format!("pending-{epoch}-{digest}.md")
}

fn inbox_has_digest(inbox: &Path, digest: &str) -> bool {
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

fn render_snapshot_md(
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
    out
}

fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cmd_hook_run(root: &Path, format: &str, args: &HookRunArgs) -> i32 {
    // Fail-open: hook runtime must never break the calling agent.
    let outcome = hook_run_inner(root, args);
    let human = super::util::is_human(format);
    match outcome {
        Ok(Receipt::Wrote { name, .. }) => {
            if args.event == "stop" {
                println!("{{\"decision\":\"allow\"}}");
            } else if human {
                println!("wrote\t{name}");
            } else {
                println!("{{\"wrote\":{name:?}}}");
            }
        }
        Ok(Receipt::Skipped { digest }) => {
            if args.event == "stop" {
                println!("{{\"decision\":\"allow\"}}");
            } else if human {
                println!("skipped\tno-change");
            } else {
                println!("{{\"skipped\":\"no-change\",\"digest\":{digest:?}}}");
            }
        }
        Err(e) => {
            eprintln!("innen hook run degraded: {e}");
            if args.event == "stop" {
                println!("{{\"decision\":\"allow\"}}");
            }
        }
    }
    0
}

enum Receipt {
    Wrote { name: String },
    Skipped { digest: String },
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
        .or_else(|| first_str(&stdin_json, &["session_id", "sessionId"]))
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
    let snap = snapshot_repo(&cwd);
    let inbox = kb.join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).map_err(|e| format!("inbox mkdir: {e}"))?;
    if inbox_has_digest(&inbox, &snap.digest) {
        return Ok(Receipt::Skipped {
            digest: snap.digest,
        });
    }
    let epoch = epoch_now();
    let name = pending_name(epoch, &snap.digest);
    let body = render_snapshot_md(&args.event, &session, &transcript, epoch, &snap);
    std::fs::write(inbox.join(&name), body).map_err(|e| format!("inbox write: {e}"))?;
    Ok(Receipt::Wrote { name })
}

// ---------------------------------------------------------------------------
// pending
// ---------------------------------------------------------------------------

fn pending_files(root: &Path) -> Vec<String> {
    let inbox = root.join("00-inbox/harvest");
    let mut names: Vec<String> = std::fs::read_dir(&inbox)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("pending-") && n.ends_with(".md"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn cmd_hook_pending(root: &Path, format: &str) -> i32 {
    let names = pending_files(root);
    if super::util::is_human(format) {
        println!("pending\t{}", names.len());
        for n in &names {
            println!("file\t{n}");
        }
    } else {
        let list = names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"pending\":[{list}]}}");
    }
    0
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || "-_./:=,".contains(c))
    {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

fn run_command(kb: &Path, event: &str) -> String {
    format!(
        "innen hook run --event {event} --kb-root {}",
        shell_quote(&kb.to_string_lossy())
    )
}

fn pending_command(kb: &Path) -> String {
    format!(
        "innen hook pending --kb-root {}",
        shell_quote(&kb.to_string_lossy())
    )
}

fn read_json_file(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn write_json_file(path: &Path, v: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut s = serde_json::to_string_pretty(v).map_err(|e| format!("serialize: {e}"))?;
    s.push('\n');
    std::fs::write(path, s).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn obj_mut<'a>(v: &'a mut serde_json::Value) -> &'a mut serde_json::Map<String, serde_json::Value> {
    if !v.is_object() {
        *v = serde_json::Value::Object(serde_json::Map::new());
    }
    v.as_object_mut().expect("object after ensure")
}

/// Append a `{matcher?, hooks:[...]}` entry's handler if no identical command
/// exists yet. Returns true when the doc changed.
fn merge_hook_entry(
    doc: &mut serde_json::Value,
    path: &[&str],
    matcher: Option<&str>,
    handler: serde_json::Value,
) -> bool {
    let mut cur = obj_mut(doc);
    for key in &path[..path.len().saturating_sub(1)] {
        let next = cur
            .entry((*key).to_string())
            .or_insert(serde_json::Value::Object(serde_json::Map::new()));
        cur = obj_mut(next);
    }
    let last = path[path.len() - 1];
    let arr = cur
        .entry(last.to_string())
        .or_insert(serde_json::Value::Array(vec![]));
    let list = arr.as_array_mut().expect("hook event holds an array");
    let want_cmd = handler
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    for entry in list.iter() {
        let entry_matcher = entry.get("matcher").and_then(|m| m.as_str());
        if entry_matcher != matcher {
            continue;
        }
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            if hooks
                .iter()
                .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(want_cmd))
            {
                return false;
            }
        }
    }
    let mut entry = serde_json::Map::new();
    if let Some(m) = matcher {
        entry.insert(
            "matcher".to_string(),
            serde_json::Value::String(m.to_string()),
        );
    }
    entry.insert("hooks".to_string(), serde_json::Value::Array(vec![handler]));
    list.push(serde_json::Value::Object(entry));
    true
}

fn command_handler(command: String, timeout: Option<u64>) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert(
        "type".to_string(),
        serde_json::Value::String("command".to_string()),
    );
    m.insert("command".to_string(), serde_json::Value::String(command));
    if let Some(t) = timeout {
        m.insert("timeout".to_string(), serde_json::Value::Number(t.into()));
    }
    serde_json::Value::Object(m)
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())
}

fn scope_dir(scope: &str, cwd: &Path, leaf: &Path) -> Result<PathBuf, String> {
    if scope == "user" {
        Ok(home_dir()?.join(leaf))
    } else {
        Ok(cwd.join(leaf))
    }
}

const OPENCODE_PLUGIN: &str = r#"/**
 * innen stop hook (opencode adapter).
 *
 * Event-driven capture only: on every model dispatch, snapshot the worktree
 * via `innen hook run` (digest-deduped, silent). First turn of each session
 * additionally reports the pending inbox count so the agent drains it.
 * Never throws; never blocks the agent.
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const KB_ROOT = "__KB_ROOT__";
const SEEN = path.join(os.homedir(), ".agents", "state", "innen-stop-hook-seen.json");

function loadSeen() {
  try {
    return JSON.parse(fs.readFileSync(SEEN, "utf8"));
  } catch {
    return {};
  }
}

function pendingCount() {
  try {
    return fs
      .readdirSync(path.join(KB_ROOT, "00-inbox", "harvest"))
      .filter((f) => f.startsWith("pending-") && f.endsWith(".md")).length;
  } catch {
    return 0;
  }
}

function capture(cwd) {
  try {
    spawnSync("innen", ["hook", "run", "--event", "session-idle", "--kb-root", KB_ROOT, "--cwd", cwd], {
      timeout: 8000,
      stdio: "ignore",
    });
  } catch {
    /* fail-open */
  }
}

export default {
  id: "innen-stop-hook",
  async setup(ctx) {
    await ctx.session.hook("context", (event) => {
      try {
        capture(process.cwd());
      } catch {
        /* fail-open */
      }
      const seen = loadSeen();
      if (seen[event.sessionID]) return;
      seen[event.sessionID] = Date.now();
      try {
        fs.mkdirSync(path.dirname(SEEN), { recursive: true });
        fs.writeFileSync(SEEN, JSON.stringify(seen));
      } catch {
        /* fail-open */
      }
      const n = pendingCount();
      event.system.push({
        type: "text",
        text: "[innen-stop-hook] Harvest inbox pending files: " + n + ". If >0, drain before new work (verify, graph/wiki, ingest, delete).",
      });
    });
  },
};
"#;

fn cmd_hook_install(root: &Path, format: &str, args: &HookInstallArgs) -> i32 {
    let human = super::util::is_human(format);
    let kb = args.kb_root.clone().unwrap_or_else(|| root.to_path_buf());
    let cwd = match &args.cwd {
        Some(p) => p.clone(),
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("innen hook install: current dir: {e}");
                return 1;
            }
        },
    };
    let res = match args.agent.as_str() {
        "claude-code" => install_claude(&kb, &args.scope, &cwd),
        "codex" => install_codex(&kb, &args.scope, &cwd),
        "grok" => install_grok(&kb, &args.scope, &cwd),
        "antigravity" => install_antigravity(&kb, &args.scope, &cwd),
        "opencode" => install_opencode(&kb, &args.scope, &cwd),
        _ => Err("unknown agent".to_string()),
    };
    match res {
        Ok(note) => {
            if human {
                println!("{note}");
            } else {
                println!("{{\"ok\":true,\"note\":{note:?}}}");
            }
            0
        }
        Err(e) => {
            eprintln!("innen hook install: {e}");
            1
        }
    }
}

fn install_claude(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".claude/settings.json")
    } else {
        scope_dir(scope, cwd, Path::new(".claude/settings.json"))?
    };
    let mut doc = read_json_file(&path);
    let run = run_command(kb, "session-end");
    let pend = pending_command(kb);
    let mut changed = false;
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        command_handler(run, None),
    );
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionStart"],
        Some("startup|resume"),
        command_handler(pend, None),
    );
    write_json_file(&path, &doc)?;
    Ok(if changed {
        format!("claude-code {scope} hooks written to {}", path.display())
    } else {
        format!(
            "claude-code {scope} hooks already present at {}",
            path.display()
        )
    })
}

fn install_codex(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".codex/hooks.json")
    } else {
        scope_dir(scope, cwd, Path::new(".codex/hooks.json"))?
    };
    let mut doc = read_json_file(&path);
    let run = run_command(kb, "session-end");
    let pend = pending_command(kb);
    let mut changed = false;
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        command_handler(run, Some(3)),
    );
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionStart"],
        Some("startup|resume"),
        command_handler(pend, Some(10)),
    );
    write_json_file(&path, &doc)?;
    Ok(format!(
        "{} Trust new hooks in Codex via /hooks before they run.",
        if changed {
            format!("codex {scope} hooks written to {}", path.display())
        } else {
            format!("codex {scope} hooks already present at {}", path.display())
        }
    ))
}

fn install_grok(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".grok/hooks/innen-progress.json")
    } else {
        scope_dir(scope, cwd, Path::new(".grok/hooks/innen-progress.json"))?
    };
    let mut doc = read_json_file(&path);
    let run = run_command(kb, "session-end");
    let pend = pending_command(kb);
    let mut changed = false;
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        command_handler(run, None),
    );
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionStart"],
        Some("startup|resume"),
        command_handler(pend, None),
    );
    write_json_file(&path, &doc)?;
    Ok(if changed {
        format!("grok {scope} hooks written to {}", path.display())
    } else {
        format!("grok {scope} hooks already present at {}", path.display())
    })
}

fn install_antigravity(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".gemini/config/hooks.json")
    } else {
        scope_dir(scope, cwd, Path::new(".agents/hooks.json"))?
    };
    let mut doc = read_json_file(&path);
    let run = run_command(kb, "stop");
    let mut key = serde_json::Map::new();
    key.insert(
        "Stop".to_string(),
        serde_json::Value::Array(vec![command_handler(run, Some(15))]),
    );
    let root = obj_mut(&mut doc);
    let prev = root.insert("innen-progress".to_string(), serde_json::Value::Object(key));
    write_json_file(&path, &doc)?;
    Ok(if prev.is_none() {
        format!(
            "antigravity {scope} Stop hook written to {}",
            path.display()
        )
    } else {
        format!(
            "antigravity {scope} Stop hook refreshed at {}",
            path.display()
        )
    })
}

fn install_opencode(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".config/opencode/plugins/innen-stop-hook.js")
    } else {
        scope_dir(
            scope,
            cwd,
            Path::new(".opencode/plugins/innen-stop-hook.js"),
        )?
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let body = OPENCODE_PLUGIN.replace(
        "__KB_ROOT__",
        &kb.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\""),
    );
    std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(format!(
        "opencode {scope} plugin written to {} (auto-loaded at startup; restart running sessions)",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        command_handler, inbox_has_digest, merge_hook_entry, pending_files, pending_name,
        render_snapshot_md, shell_quote, Snapshot,
    };
    use std::collections::HashSet;

    fn sample_snap() -> Snapshot {
        Snapshot {
            repo: "/repo".to_string(),
            branch: "main".to_string(),
            dirty_count: 21,
            digest: "abc123def456".to_string(),
            sample: vec![" M a".to_string(), "?? b".to_string()],
        }
    }

    #[test]
    fn pending_name_pins_epoch_and_digest() {
        assert_eq!(pending_name(7, "abc123def456"), "pending-7-abc123def456.md");
    }

    #[test]
    fn render_caps_sample_and_marks_overflow() {
        let mut snap = sample_snap();
        snap.sample = (0..20).map(|i| format!(" M f{i}")).collect();
        snap.dirty_count = 30;
        let md = render_snapshot_md("session-end", "s1", "-", 9, &snap);
        assert!(md.contains("- event: session-end"));
        assert!(md.contains("- digest: abc123def456"));
        assert!(md.contains("(10 more)"));
        assert!(!md.contains(" M f20"));
    }

    #[test]
    fn merge_appends_without_duplicating_command() {
        let mut doc = serde_json::Value::Null;
        let h = command_handler("innen hook run --event session-end".to_string(), None);
        assert!(merge_hook_entry(
            &mut doc,
            &["hooks", "SessionEnd"],
            None,
            h.clone()
        ));
        assert!(!merge_hook_entry(
            &mut doc,
            &["hooks", "SessionEnd"],
            None,
            h
        ));
        let hooks = doc["hooks"]["SessionEnd"].as_array().expect("array");
        assert_eq!(hooks.len(), 1);
        assert_eq!(
            hooks[0]["hooks"][0]["command"],
            serde_json::Value::String("innen hook run --event session-end".to_string())
        );
    }

    #[test]
    fn merge_keeps_unrelated_entries_and_matchers_apart() {
        let mut doc = serde_json::json!({"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "other"}]}]}});
        let h = command_handler("innen hook run --event session-end".to_string(), Some(3));
        assert!(merge_hook_entry(
            &mut doc,
            &["hooks", "SessionEnd"],
            None,
            h
        ));
        assert_eq!(
            doc["hooks"]["SessionEnd"].as_array().expect("array").len(),
            2
        );
        let mut seen = HashSet::new();
        for e in doc["hooks"]["SessionEnd"].as_array().expect("array") {
            for hh in e["hooks"].as_array().expect("hooks") {
                seen.insert(hh["command"].as_str().expect("cmd").to_string());
            }
        }
        assert!(seen.contains("other"));
        assert!(seen.contains("innen hook run --event session-end"));
    }

    #[test]
    fn stop_output_contract_is_exact() {
        // Antigravity Stop requires a `decision` field; anything else allows it.
        let v = serde_json::json!({"decision": "allow"});
        assert_eq!(
            serde_json::to_string(&v).expect("json"),
            "{\"decision\":\"allow\"}"
        );
    }

    #[test]
    fn shell_quote_pins_safe_and_spaced_paths() {
        assert_eq!(shell_quote("/a/b-c"), "/a/b-c");
        assert_eq!(shell_quote("/a b"), "\"/a b\"");
    }

    #[test]
    fn inbox_digest_and_pending_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inbox = dir.path().join("00-inbox/harvest");
        std::fs::create_dir_all(&inbox).expect("mkdir");
        assert!(pending_files(dir.path()).is_empty());
        assert!(!inbox_has_digest(&inbox, "abc123def456"));
        std::fs::write(inbox.join("pending-9-abc123def456.md"), "x").expect("write");
        std::fs::write(inbox.join("notes.md"), "y").expect("write");
        assert!(inbox_has_digest(&inbox, "abc123def456"));
        assert_eq!(
            pending_files(dir.path()),
            vec!["pending-9-abc123def456.md".to_string()]
        );
    }
}

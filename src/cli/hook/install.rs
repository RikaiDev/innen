//! Writing each agent's hook config: path resolution, JSON merge helpers,
//! and dispatch.

use std::path::{Path, PathBuf};

use super::agents::{
    install_antigravity, install_claude, install_codex, install_grok, install_opencode,
};
use super::HookInstallArgs;
use crate::cli::util::is_human;
// Shared with `agents` (the opencode template) and `tests`.
pub(super) use super::plugin::OPENCODE_PLUGIN;

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

pub(super) fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || "-_./:=,".contains(c))
    {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

pub(super) fn run_command(kb: &Path, event: &str) -> String {
    let executable = std::env::current_exe()
        .ok()
        .map(|p| shell_quote(&p.to_string_lossy()))
        .unwrap_or_else(|| "innen".to_string());
    format!(
        "{executable} hook run --event {event} --kb-root {}",
        shell_quote(&kb.to_string_lossy())
    )
}

pub(super) fn pending_command(kb: &Path) -> String {
    let executable = std::env::current_exe()
        .ok()
        .map(|p| shell_quote(&p.to_string_lossy()))
        .unwrap_or_else(|| "innen".to_string());
    format!(
        "{executable} hook pending --kb-root {}",
        shell_quote(&kb.to_string_lossy())
    )
}

pub(super) fn read_json_file(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

pub(super) fn write_json_file(path: &Path, v: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut s = serde_json::to_string_pretty(v).map_err(|e| format!("serialize: {e}"))?;
    s.push('\n');
    std::fs::write(path, s).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

pub(super) fn obj_mut(
    v: &mut serde_json::Value,
) -> &mut serde_json::Map<String, serde_json::Value> {
    if !v.is_object() {
        *v = serde_json::Value::Object(serde_json::Map::new());
    }
    v.as_object_mut().expect("object after ensure")
}

/// Append a `{matcher?, hooks:[...]}` entry's handler if no identical command
/// exists yet. Returns true when the doc changed.
pub(super) fn merge_hook_entry(
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

/// Remove handlers installed by an earlier innen binary while preserving
/// unrelated handlers in the same event group.
pub(super) fn remove_hook_commands(doc: &mut serde_json::Value, event: &str, marker: &str) -> bool {
    let Some(root) = doc.as_object_mut() else {
        return false;
    };
    let Some(events) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    let Some(entries) = events.get_mut(event).and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for entry in entries.iter_mut() {
        if let Some(handlers) = entry.get_mut("hooks").and_then(|v| v.as_array_mut()) {
            let before = handlers.len();
            handlers.retain(|handler| {
                !handler
                    .get("command")
                    .and_then(|v| v.as_str())
                    .is_some_and(|command| command.contains(marker))
            });
            changed |= before != handlers.len();
        }
    }
    let before = entries.len();
    entries.retain(|entry| {
        entry
            .get("hooks")
            .and_then(|v| v.as_array())
            .is_none_or(|handlers| !handlers.is_empty())
    });
    changed || before != entries.len()
}

pub(super) fn command_handler(command: String, timeout: Option<u64>) -> serde_json::Value {
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

pub(super) fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())
}

pub(super) fn scope_dir(scope: &str, cwd: &Path, leaf: &Path) -> Result<PathBuf, String> {
    if scope == "user" {
        Ok(home_dir()?.join(leaf))
    } else {
        Ok(cwd.join(leaf))
    }
}

pub(super) fn cmd_hook_install(root: &Path, format: &str, args: &HookInstallArgs) -> i32 {
    let human = is_human(format);
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

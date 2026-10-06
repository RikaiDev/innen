//! Per-agent wiring: where each coding tool keeps its hook config.

use std::path::Path;

use super::install::{
    command_handler, home_dir, merge_hook_entry, obj_mut, pending_command, read_json_file,
    remove_hook_commands, run_command, scope_dir, write_json_file, OPENCODE_PLUGIN,
};
pub(super) fn install_claude(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".claude/settings.json")
    } else {
        scope_dir(scope, cwd, Path::new(".claude/settings.json"))?
    };
    let mut doc = read_json_file(&path);
    let mut changed = false;
    changed |= remove_hook_commands(&mut doc, "SessionEnd", "hook run --event session-end");
    changed |= remove_hook_commands(&mut doc, "SessionStart", "hook pending");
    changed |= remove_hook_commands(&mut doc, "PreCompact", "hook run --event compact");
    let run = run_command(kb, "session-end");
    let compact = run_command(kb, "compact");
    let pend = pending_command(kb);
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "SessionEnd"],
        None,
        command_handler(run, None),
    );
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "PreCompact"],
        None,
        command_handler(compact, None),
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

pub(super) fn install_codex(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
    let path = if scope == "user" {
        home_dir()?.join(".codex/hooks.json")
    } else {
        scope_dir(scope, cwd, Path::new(".codex/hooks.json"))?
    };
    let mut doc = read_json_file(&path);
    let mut changed = false;
    changed |= remove_hook_commands(&mut doc, "SessionEnd", "hook run --event session-end");
    changed |= remove_hook_commands(&mut doc, "SessionStart", "hook pending");
    changed |= remove_hook_commands(&mut doc, "Stop", "hook run --event stop");
    let run = run_command(kb, "session-end");
    let stop = run_command(kb, "stop");
    let pend = pending_command(kb);
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
    changed |= merge_hook_entry(
        &mut doc,
        &["hooks", "Stop"],
        None,
        command_handler(stop, Some(10)),
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

pub(super) fn install_grok(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
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

pub(super) fn install_antigravity(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
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

pub(super) fn install_opencode(kb: &Path, scope: &str, cwd: &Path) -> Result<String, String> {
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

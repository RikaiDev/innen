//! Config precedence + minimal TOML reader (Task 8c, P1).
//!
//! Precedence (pinned): CLI flags > `INNEN_` env > `.innen/machine.json` >
//! `innen.toml` > builtin.
//!
//! Only keys: `[core] root, format`; `[index] rebuild_on_open`. Unknown
//! sections (incl. `[mcp]`) and unknown keys are ignored in P1.
//!
//! API: [`load`] reads real process env and returns
//! `(Config, Vec<String> /*warnings*/)`; [`load_with_env`] is the same merge
//! with an injected env map so tests avoid process-env races.
//!
//! Env names: `INNEN_ROOT`, `INNEN_FORMAT`, `INNEN_REBUILD_ON_OPEN`
//! (`true`/`false`, case-insensitive; invalid values ignored; empty strings
//! for `root`/`format` ignored).
//!
//! File layout: `innen.toml` at `<root>/innen.toml`; `machine.json` at
//! `<root>/.innen/machine.json` as a flat JSON object
//! (`{"root","format","rebuild_on_open"}`; unknown keys / wrong types ignored).
//! `machine.json` records only CLI-explicit keys (written by the CLI layer;
//! this module only reads it). When the same key appears in both files the
//! machine layer wins and a warning is emitted exactly as
//! `warning: machine.json overrides innen.toml: <key>` where `<key>` is the
//! bare key name (`root` | `format` | `rebuild_on_open`), in fixed order
//! `root, format, rebuild_on_open`. The warning is emitted whenever both files
//! set the key, regardless of whether a higher layer (env/CLI) later wins.
//!
//! Builtins: `root` = the `root` argument passed to [`load`],
//! `format` = `"human"`, `rebuild_on_open` = `false`. File lookup always uses
//! the passed-in `root` argument (the configured `root` value does not
//! re-target lookup, avoiding a bootstrap paradox).
//!
//! Minimal TOML limits (no TOML parser in workspace deps, none added):
//! only `[core]` / `[index]` sections; only `key = "value"` (double-quoted)
//! for `root`/`format` and bare `key = true|false` (exact lowercase) for
//! `rebuild_on_open`; `#` starts a comment outside double quotes; whitespace
//! trimmed; section headers are exact `[core]` / `[index]` (no dotted keys, no
//! inline tables, no arrays, no single quotes, no multi-line strings, no
//! escapes beyond `\\` and `\"`). Anything else is ignored. Unreadable or
//! missing files are treated as empty (no error).
//!
//! Env-race note: Rust tests share one process, so tests that touch real
//! process env can race. Only `config_precedence_env_over_toml` touches real
//! env (key `INNEN_FORMAT`, removed after); the other config test uses
//! [`load_with_env`] with an injected empty env map and the lint test touches
//! no env, so no two tests contend on the same real env key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// CLI-explicit overrides (highest precedence layer).
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    /// Overrides `[core] root`.
    pub root: Option<PathBuf>,
    /// Overrides `[core] format`.
    pub format: Option<String>,
    /// Overrides `[index] rebuild_on_open`.
    pub rebuild_on_open: Option<bool>,
}

/// Resolved P1 config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Data root. Builtin default = the `root` argument to [`load`].
    pub root: PathBuf,
    /// Output format (e.g. `"human"` / `"json"`). Builtin = `"human"`.
    pub format: String,
    /// Rebuild the derived index on open. Builtin = `false`.
    pub rebuild_on_open: bool,
}

/// One layer's parsed values; `None` = key absent (or invalid/ignored).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Layer {
    root: Option<PathBuf>,
    format: Option<String>,
    rebuild_on_open: Option<bool>,
}

/// Strip a `#` comment that appears outside double quotes. Handles `\"` and
/// `\\` escapes minimally so a `#` inside a quoted value survives.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_quotes => {
                // Skip the escaped char (e.g. `\"` must not toggle quoting).
                i += 2;
                continue;
            }
            b'"' => {
                in_quotes = !in_quotes;
            }
            b'#' if !in_quotes => return line[..i].trim_end(),
            _ => {}
        }
        i += 1;
    }
    line
}

/// Unquote a double-quoted value; supports `\\` and `\"` only. Returns `None`
/// when the value is not exactly one double-quoted string. Interior bare
/// quotes (`"""`, `"a"b"`) are rejected.
fn unquote(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            }
        } else if c == '"' {
            // Bare interior quote: not an escape, not the outer pair.
            return None;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Parse minimal P1 TOML text into a [`Layer`]. See module docs for limits.
fn parse_toml_text(text: &str) -> Layer {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Section {
        Core,
        Index,
        Other,
    }
    // Strip a leading BOM so `\uFEFF[core]` still parses as a header.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut section: Option<Section> = None;
    let mut layer = Layer::default();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            // Section header: exact `[core]` / `[index]` after trimming inner
            // whitespace; anything else (incl. `[mcp]`) becomes Other.
            // Malformed headers (`[core] extra`, unclosed `[core`) reset to
            // the ignore-section instead of retaining the prior section.
            if line.ends_with(']') {
                let name = line[1..line.len() - 1].trim();
                section = Some(match name {
                    "core" => Section::Core,
                    "index" => Section::Index,
                    _ => Section::Other,
                });
            } else {
                section = Some(Section::Other);
            }
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();
        match (section, key) {
            (Some(Section::Core), "root") => {
                if let Some(s) = unquote(value) {
                    // Empty `""` is ignored (falls through to lower layer),
                    // matching empty-env handling.
                    if !s.is_empty() {
                        layer.root = Some(PathBuf::from(s));
                    }
                }
            }
            (Some(Section::Core), "format") => {
                if let Some(s) = unquote(value) {
                    if !s.is_empty() {
                        layer.format = Some(s);
                    }
                }
            }
            (Some(Section::Index), "rebuild_on_open") => match value {
                "true" => layer.rebuild_on_open = Some(true),
                "false" => layer.rebuild_on_open = Some(false),
                _ => {}
            },
            _ => {}
        }
    }
    layer
}

/// Read and parse `<root>/innen.toml`; missing/unreadable → empty layer.
fn read_toml_layer(root: &Path) -> Layer {
    let text = match std::fs::read_to_string(root.join("innen.toml")) {
        Ok(text) => text,
        Err(_) => return Layer::default(),
    };
    parse_toml_text(&text)
}

/// Parse flat `machine.json` bytes; unknown keys / wrong types ignored,
/// any parse failure → empty layer. A leading UTF-8 BOM is stripped;
/// empty-string values are ignored (fall through to lower layers).
fn parse_machine_bytes(bytes: &[u8]) -> Layer {
    // Strip UTF-8 BOM (`EF BB BF`) when present.
    let bytes = bytes
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(bytes);
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return Layer::default(),
    };
    let Some(obj) = value.as_object() else {
        return Layer::default();
    };
    let mut layer = Layer::default();
    if let Some(s) = obj.get("root").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            layer.root = Some(PathBuf::from(s));
        }
    }
    if let Some(s) = obj.get("format").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            layer.format = Some(s.to_string());
        }
    }
    if let Some(b) = obj.get("rebuild_on_open").and_then(|v| v.as_bool()) {
        layer.rebuild_on_open = Some(b);
    }
    layer
}

/// Read and parse `<root>/.innen/machine.json`; missing/unreadable → empty.
fn read_machine_layer(root: &Path) -> Layer {
    let bytes = match std::fs::read(root.join(".innen").join("machine.json")) {
        Ok(bytes) => bytes,
        Err(_) => return Layer::default(),
    };
    parse_machine_bytes(&bytes)
}

/// Parse `INNEN_REBUILD_ON_OPEN` values (`true`/`false`, case-insensitive).
fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Merge layers in precedence order and collect machine-over-toml warnings.
fn merge(
    root_arg: &Path,
    toml: &Layer,
    machine: &Layer,
    env_root: Option<PathBuf>,
    env_format: Option<String>,
    env_rebuild: Option<bool>,
    cli: &CliOverrides,
) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    for key in ["root", "format", "rebuild_on_open"] {
        let both = match key {
            "root" => toml.root.is_some() && machine.root.is_some(),
            "format" => toml.format.is_some() && machine.format.is_some(),
            _ => toml.rebuild_on_open.is_some() && machine.rebuild_on_open.is_some(),
        };
        if both {
            warnings.push(format!("warning: machine.json overrides innen.toml: {key}"));
        }
    }

    let mut cfg = Config {
        root: root_arg.to_path_buf(),
        format: "human".to_string(),
        rebuild_on_open: false,
    };
    if let Some(v) = &toml.root {
        cfg.root = v.clone();
    }
    if let Some(v) = &toml.format {
        cfg.format = v.clone();
    }
    if let Some(v) = toml.rebuild_on_open {
        cfg.rebuild_on_open = v;
    }
    if let Some(v) = &machine.root {
        cfg.root = v.clone();
    }
    if let Some(v) = &machine.format {
        cfg.format = v.clone();
    }
    if let Some(v) = machine.rebuild_on_open {
        cfg.rebuild_on_open = v;
    }
    if let Some(v) = env_root {
        cfg.root = v;
    }
    if let Some(v) = env_format {
        cfg.format = v;
    }
    if let Some(v) = env_rebuild {
        cfg.rebuild_on_open = v;
    }
    if let Some(v) = &cli.root {
        cfg.root = v.clone();
    }
    if let Some(v) = &cli.format {
        cfg.format = v.clone();
    }
    if let Some(v) = cli.rebuild_on_open {
        cfg.rebuild_on_open = v;
    }
    (cfg, warnings)
}

/// Shared core: merge file layers with already-extracted env values.
fn load_impl(
    root: &Path,
    cli: &CliOverrides,
    env_root: Option<PathBuf>,
    env_format: Option<String>,
    env_rebuild: Option<bool>,
) -> (Config, Vec<String>) {
    let toml = read_toml_layer(root);
    let machine = read_machine_layer(root);
    merge(
        root,
        &toml,
        &machine,
        env_root,
        env_format,
        env_rebuild,
        cli,
    )
}

/// Load config for `root`, reading real process env.
///
/// Returns the resolved [`Config`] plus warnings (each exactly
/// `warning: machine.json overrides innen.toml: <key>`).
pub fn load(root: &Path, cli: &CliOverrides) -> (Config, Vec<String>) {
    let env_root = std::env::var("INNEN_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    let env_format = std::env::var("INNEN_FORMAT")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_rebuild = std::env::var("INNEN_REBUILD_ON_OPEN")
        .ok()
        .and_then(|s| parse_env_bool(&s));
    load_impl(root, cli, env_root, env_format, env_rebuild)
}

/// Same merge as [`load`] with an injected env map (test hook; avoids
/// process-env races). `env` maps `INNEN_*` names to values.
pub fn load_with_env(
    root: &Path,
    cli: &CliOverrides,
    env: &HashMap<String, String>,
) -> (Config, Vec<String>) {
    let env_root = env
        .get("INNEN_ROOT")
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    let env_format = env
        .get("INNEN_FORMAT")
        .filter(|s| !s.trim().is_empty())
        .cloned();
    let env_rebuild = env
        .get("INNEN_REBUILD_ON_OPEN")
        .and_then(|s| parse_env_bool(s));
    load_impl(root, cli, env_root, env_format, env_rebuild)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn config_precedence_env_over_toml() {
        // innen.toml sets format=json, env INNEN_FORMAT=human → human wins.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("innen.toml"), "[core]\nformat = \"json\"\n")
            .expect("write innen.toml");
        std::env::set_var("INNEN_FORMAT", "human");
        let (cfg, _warnings) = load(dir.path(), &CliOverrides::default());
        std::env::remove_var("INNEN_FORMAT");
        assert_eq!(cfg.format, "human", "env must beat toml: {cfg:?}");
    }

    #[test]
    fn config_machine_warns_on_override() {
        // machine.json sets format + toml sets format → warning string emitted.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("innen.toml"), "[core]\nformat = \"json\"\n")
            .expect("write innen.toml");
        let innen_dir = dir.path().join(".innen");
        fs::create_dir_all(&innen_dir).expect("mkdir .innen");
        fs::write(innen_dir.join("machine.json"), r#"{"format": "human"}"#)
            .expect("write machine.json");
        let env: HashMap<String, String> = HashMap::new();
        let (cfg, warnings) = load_with_env(dir.path(), &CliOverrides::default(), &env);
        assert_eq!(cfg.format, "human", "machine.json must beat toml: {cfg:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w == "warning: machine.json overrides innen.toml: format"),
            "must emit exact override warning, got: {warnings:?}"
        );
    }
}

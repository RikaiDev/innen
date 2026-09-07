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

use std::collections::{BTreeMap, HashMap};
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

/// CLI `config set` failure, extracted from `src/main.rs`.
///
/// Placement choice: machine.json persistence lives here in `config.rs` (not a
/// new module) to share the flat-object layout and sorted-keys discipline with
/// the read path above; graph write-path rules live in `graph.rs`. Display
/// strings match the former binary `eprintln!("error: {e}")` suffixes exactly.
#[derive(Debug, thiserror::Error)]
pub enum ConfigSetError {
    #[error("unknown config key: {0} (root|format|rebuild_on_open)")]
    UnknownKey(String),
    #[error("rebuild_on_open must be true|false")]
    InvalidBool,
    #[error("{0} must be non-empty")]
    EmptyValue(String),
    #[error("root must be an absolute path: {0}")]
    NotAbsolute(String),
    #[error("{var} must be an absolute path: {path}")]
    RelativeConfigHome { var: &'static str, path: PathBuf },
    #[error("corrupt global config at {}: {detail}", path.display())]
    CorruptGlobal { path: PathBuf, detail: String },
    #[error("global configuration directory could not be determined; set HOME or XDG_CONFIG_HOME")]
    NoGlobalDir,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Global config directory:
/// 1. `$XDG_CONFIG_HOME/innen` if `XDG_CONFIG_HOME` is non-empty (must be absolute)
/// 2. Else `$HOME/.config/innen` if `HOME` is non-empty (must be absolute)
pub fn global_config_dir() -> Result<PathBuf, RootResolutionError> {
    global_config_dir_with_env(None)
}

/// Global config directory with optional injected env map for tests.
pub fn global_config_dir_with_env(
    env: Option<&HashMap<String, String>>,
) -> Result<PathBuf, RootResolutionError> {
    let get_var = |k: &str| -> Option<String> {
        if let Some(map) = env {
            map.get(k).cloned()
        } else {
            std::env::var(k).ok()
        }
    };
    if let Some(val) = get_var("XDG_CONFIG_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if !p.is_absolute() {
                return Err(RootResolutionError::RelativeConfigHome {
                    var: "XDG_CONFIG_HOME",
                    path: p,
                });
            }
            return Ok(p.join("innen"));
        }
    }
    if let Some(val) = get_var("HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if !p.is_absolute() {
                return Err(RootResolutionError::RelativeConfigHome {
                    var: "HOME",
                    path: p,
                });
            }
            return Ok(p.join(".config").join("innen"));
        }
    }
    Err(RootResolutionError::NoGlobalDir)
}

/// Canonical global config file path: `<global_config_dir>/config.json`.
pub fn global_config_path() -> Result<PathBuf, RootResolutionError> {
    global_config_path_with_env(None)
}

/// Global config file path with optional injected env map for tests.
pub fn global_config_path_with_env(
    env: Option<&HashMap<String, String>>,
) -> Result<PathBuf, RootResolutionError> {
    let dir = global_config_dir_with_env(env)?;
    Ok(dir.join("config.json"))
}

/// Validate `key`/`value` and persist one key into the global user config file.
///
/// Requires `root` to be an absolute path, normalizes consistently, and preserves unrelated keys.
pub fn set_global_value(key: &str, value: &str) -> Result<String, ConfigSetError> {
    set_global_value_with_env(key, value, None)
}

/// Injected-env variant of [`set_global_value`].
pub fn set_global_value_with_env(
    key: &str,
    value: &str,
    env: Option<&HashMap<String, String>>,
) -> Result<String, ConfigSetError> {
    if !matches!(key, "root" | "format" | "rebuild_on_open") {
        return Err(ConfigSetError::UnknownKey(key.to_string()));
    }
    let normalized = value.trim();
    if (key == "root" || key == "format") && normalized.is_empty() {
        return Err(ConfigSetError::EmptyValue(key.to_string()));
    }
    if key == "rebuild_on_open"
        && !matches!(normalized.to_ascii_lowercase().as_str(), "true" | "false")
    {
        return Err(ConfigSetError::InvalidBool);
    }
    if key == "root" && !Path::new(normalized).is_absolute() {
        return Err(ConfigSetError::NotAbsolute(normalized.to_string()));
    }
    let (stored, echo) = if key == "rebuild_on_open" {
        let b = normalized.eq_ignore_ascii_case("true");
        (serde_json::Value::Bool(b), normalized.to_ascii_lowercase())
    } else {
        (
            serde_json::Value::String(normalized.to_string()),
            normalized.to_string(),
        )
    };
    let global_path = global_config_path_with_env(env).map_err(|e| match e {
        RootResolutionError::NoGlobalDir => ConfigSetError::NoGlobalDir,
        RootResolutionError::RelativeConfigHome { var, path } => {
            ConfigSetError::RelativeConfigHome { var, path }
        }
        _ => ConfigSetError::CorruptGlobal {
            path: PathBuf::new(),
            detail: e.to_string(),
        },
    })?;
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut map: BTreeMap<String, serde_json::Value> = match std::fs::read(&global_path) {
        Ok(bytes) => {
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            serde_json::from_slice(bytes).map_err(|e| ConfigSetError::CorruptGlobal {
                path: global_path.clone(),
                detail: e.to_string(),
            })?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(ConfigSetError::Io(e)),
    };
    map.insert(key.to_string(), stored);
    let text = serde_json::to_string(&map).expect("global config serializes");
    std::fs::write(&global_path, format!("{text}\n"))?;
    Ok(echo)
}

/// Read one value from global user configuration.
pub fn get_global_value(key: &str) -> Result<Option<String>, ConfigSetError> {
    get_global_value_with_env(key, None)
}

/// Injected-env variant of [`get_global_value`].
pub fn get_global_value_with_env(
    key: &str,
    env: Option<&HashMap<String, String>>,
) -> Result<Option<String>, ConfigSetError> {
    if !matches!(key, "root" | "format" | "rebuild_on_open") {
        return Err(ConfigSetError::UnknownKey(key.to_string()));
    }
    let global_path = match global_config_path_with_env(env) {
        Ok(p) => p,
        Err(RootResolutionError::NoGlobalDir) => return Ok(None),
        Err(RootResolutionError::RelativeConfigHome { var, path }) => {
            return Err(ConfigSetError::RelativeConfigHome { var, path });
        }
        Err(e) => {
            return Err(ConfigSetError::CorruptGlobal {
                path: PathBuf::new(),
                detail: e.to_string(),
            });
        }
    };
    let bytes = match std::fs::read(&global_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ConfigSetError::Io(e)),
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let val: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| ConfigSetError::CorruptGlobal {
            path: global_path.clone(),
            detail: e.to_string(),
        })?;
    let Some(obj) = val.as_object() else {
        return Err(ConfigSetError::CorruptGlobal {
            path: global_path,
            detail: "expected JSON object".to_string(),
        });
    };
    match key {
        "root" => Ok(obj
            .get("root")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())),
        "format" => Ok(obj
            .get("format")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())),
        "rebuild_on_open" => Ok(obj
            .get("rebuild_on_open")
            .and_then(|v| v.as_bool())
            .map(|b| b.to_string())),
        _ => Ok(None),
    }
}

/// Root resolution failure.
#[derive(Debug, thiserror::Error)]
pub enum RootResolutionError {
    #[error("knowledge base root is not configured; set --root, INNEN_ROOT, or register globally with 'innen config set --global root <path>'")]
    Unconfigured,
    #[error("INNEN_ROOT must be an absolute path: {}", .0.display())]
    RelativeEnvRoot(PathBuf),
    #[error("'root' in global config must be an absolute path: {}", .0.display())]
    RelativeGlobalRoot(PathBuf),
    #[error("{var} must be an absolute path: {}", path.display())]
    RelativeConfigHome { var: &'static str, path: PathBuf },
    #[error("corrupt global config at {}: {detail}", path.display())]
    CorruptConfig { path: PathBuf, detail: String },
    #[error("unreadable global config at {}: {detail}", path.display())]
    UnreadableConfig { path: PathBuf, detail: String },
    #[error("global configuration directory could not be determined; set HOME or XDG_CONFIG_HOME")]
    NoGlobalDir,
}

/// KB root resolution: `--root <dir>` > `INNEN_ROOT` env > user-level persisted root.
///
/// Fails with an actionable error if unconfigured. Never silently falls back to cwd.
/// Explicit `--root` is a deliberate caller scope override.
/// INNEN_ROOT and global persisted root require absolute paths.
pub fn resolve_root(cli_root: Option<PathBuf>) -> Result<PathBuf, RootResolutionError> {
    resolve_root_with_env(cli_root, None)
}

/// Injected-env variant of [`resolve_root`] for tests.
pub fn resolve_root_with_env(
    cli_root: Option<PathBuf>,
    env: Option<&HashMap<String, String>>,
) -> Result<PathBuf, RootResolutionError> {
    // 1. Explicit CLI --root flag wins (deliberate scope override)
    if let Some(r) = cli_root {
        if !r.as_os_str().is_empty() {
            return Ok(r);
        }
    }
    // 2. INNEN_ROOT env (non-empty; must be absolute)
    let env_root = if let Some(map) = env {
        map.get("INNEN_ROOT").cloned()
    } else {
        std::env::var("INNEN_ROOT").ok()
    };
    if let Some(s) = env_root {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if !p.is_absolute() {
                return Err(RootResolutionError::RelativeEnvRoot(p));
            }
            return Ok(p);
        }
    }
    // 3. User-level persisted root in canonical config.json
    let config_path = match global_config_path_with_env(env) {
        Ok(p) => p,
        Err(RootResolutionError::NoGlobalDir) => return Err(RootResolutionError::Unconfigured),
        Err(e) => return Err(e),
    };
    let bytes = match std::fs::read(&config_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RootResolutionError::Unconfigured);
        }
        Err(e) => {
            return Err(RootResolutionError::UnreadableConfig {
                path: config_path,
                detail: e.to_string(),
            });
        }
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let val: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| RootResolutionError::CorruptConfig {
            path: config_path.clone(),
            detail: e.to_string(),
        })?;
    let Some(obj) = val.as_object() else {
        return Err(RootResolutionError::CorruptConfig {
            path: config_path,
            detail: "expected JSON object at root".to_string(),
        });
    };
    if let Some(root_val) = obj.get("root") {
        if let Some(s) = root_val.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                if !p.is_absolute() {
                    return Err(RootResolutionError::RelativeGlobalRoot(p));
                }
                return Ok(p);
            }
        }
        return Err(RootResolutionError::CorruptConfig {
            path: config_path,
            detail: "'root' value must be a non-empty string".to_string(),
        });
    }

    // 4. No silent cwd fallback!
    Err(RootResolutionError::Unconfigured)
}

/// Validate `key`/`value` and persist one key into `<root>/.innen/machine.json`.
///
/// Mirrors the former binary `cmd_config_set` byte-for-byte: key allowlist,
/// `rebuild_on_open` true|false check, non-empty check for `root`/`format`,
/// `Bool` vs `String` stored shape, `create_dir_all`, read-merge-write with
/// sorted keys plus trailing newline (unparseable bytes fall back to empty;
/// only `NotFound` is a clean empty — other reads fail). Returns the echo
/// string (`rebuild_on_open` lowercased, others verbatim) for the caller to print.
pub fn set_machine_value(root: &Path, key: &str, value: &str) -> Result<String, ConfigSetError> {
    if !matches!(key, "root" | "format" | "rebuild_on_open") {
        return Err(ConfigSetError::UnknownKey(key.to_string()));
    }
    if key == "rebuild_on_open"
        && !matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "false")
    {
        return Err(ConfigSetError::InvalidBool);
    }
    if (key == "root" || key == "format") && value.trim().is_empty() {
        return Err(ConfigSetError::EmptyValue(key.to_string()));
    }
    let stored = if key == "rebuild_on_open" {
        let b = value.trim().eq_ignore_ascii_case("true");
        serde_json::Value::Bool(b)
    } else {
        serde_json::Value::String(value.to_string())
    };
    let machine_path = root.join(".innen").join("machine.json");
    std::fs::create_dir_all(root.join(".innen"))?;
    let mut map: BTreeMap<String, serde_json::Value> = match std::fs::read(&machine_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(ConfigSetError::Io(e)),
    };
    map.insert(key.to_string(), stored);
    let text = serde_json::to_string(&map).expect("machine.json serializes");
    std::fs::write(&machine_path, format!("{text}\n"))?;
    let echo = match key {
        "rebuild_on_open" => value.trim().to_ascii_lowercase(),
        _ => value.to_string(),
    };
    Ok(echo)
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

    #[test]
    fn resolve_root_precedence_and_error_cases() {
        let home = tempfile::tempdir().expect("tempdir");
        let kb_dir = tempfile::tempdir().expect("tempdir");
        let kb_path = kb_dir.path().to_path_buf();
        let mut env = HashMap::new();
        env.insert(
            "XDG_CONFIG_HOME".to_string(),
            home.path().to_string_lossy().into_owned(),
        );

        // 1. Unconfigured fails without mutation
        let err = resolve_root_with_env(None, Some(&env)).unwrap_err();
        assert!(matches!(err, RootResolutionError::Unconfigured));

        // 2. Global bootstrap registration requires absolute path
        let rel_err = set_global_value_with_env("root", "relative/path", Some(&env)).unwrap_err();
        assert!(matches!(rel_err, ConfigSetError::NotAbsolute(_)));

        // 3. Global bootstrap sets root and preserves unrelated keys
        set_global_value_with_env("format", "json", Some(&env)).expect("set format");
        set_global_value_with_env("root", &kb_path.to_string_lossy(), Some(&env))
            .expect("set root");
        let got_root = resolve_root_with_env(None, Some(&env)).expect("resolve root");
        assert_eq!(got_root, kb_path);
        let format_val = get_global_value_with_env("format", Some(&env))
            .expect("get format")
            .expect("format present");
        assert_eq!(format_val, "json");

        // 4. INNEN_ROOT overrides global config
        let override_kb = tempfile::tempdir().expect("tempdir");
        env.insert(
            "INNEN_ROOT".to_string(),
            override_kb.path().to_string_lossy().into_owned(),
        );
        let got_override = resolve_root_with_env(None, Some(&env)).expect("env override");
        assert_eq!(got_override, override_kb.path());

        // 5. CLI --root overrides INNEN_ROOT and global config
        let cli_kb = tempfile::tempdir().expect("tempdir");
        let got_cli =
            resolve_root_with_env(Some(cli_kb.path().to_path_buf()), Some(&env)).expect("cli");
        assert_eq!(got_cli, cli_kb.path());

        // 6. Malformed global config fails explicitly
        env.remove("INNEN_ROOT");
        let cfg_file = home.path().join("innen").join("config.json");
        fs::write(&cfg_file, "{corrupt json").expect("corrupt write");
        let corrupt_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
        assert!(matches!(
            corrupt_err,
            RootResolutionError::CorruptConfig { .. }
        ));

        // 7. Relative root in global config fails explicitly
        fs::write(&cfg_file, r#"{"root": "relative/path"}"#).expect("write relative root");
        let rel_global_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
        assert!(matches!(
            rel_global_err,
            RootResolutionError::RelativeGlobalRoot(_)
        ));

        // 8. Relative INNEN_ROOT fails explicitly
        env.insert("INNEN_ROOT".to_string(), "relative/kb".to_string());
        let rel_env_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
        assert!(matches!(
            rel_env_err,
            RootResolutionError::RelativeEnvRoot(_)
        ));
    }
}

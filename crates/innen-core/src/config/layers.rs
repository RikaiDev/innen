//! Layer readers: the CLI, env, machine.json and innen.toml inputs.

use std::path::{Path, PathBuf};

use crate::toml::{strip_comment, unquote};

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
pub(super) struct Layer {
    pub(super) root: Option<PathBuf>,
    pub(super) format: Option<String>,
    pub(super) rebuild_on_open: Option<bool>,
}

/// Parse minimal P1 TOML text into a [`Layer`]. See module docs for limits.
///
/// Comment stripping and value unquoting live in [`crate::toml`], shared with
/// `profile.toml` so the two readers cannot drift.
pub(super) fn parse_toml_text(text: &str) -> Layer {
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
pub(super) fn read_toml_layer(root: &Path) -> Layer {
    let text = match std::fs::read_to_string(root.join("innen.toml")) {
        Ok(text) => text,
        Err(_) => return Layer::default(),
    };
    parse_toml_text(&text)
}

/// Parse flat `machine.json` bytes; unknown keys / wrong types ignored,
/// any parse failure → empty layer. A leading UTF-8 BOM is stripped;
/// empty-string values are ignored (fall through to lower layers).
pub(super) fn parse_machine_bytes(bytes: &[u8]) -> Layer {
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
pub(super) fn read_machine_layer(root: &Path) -> Layer {
    let bytes = match std::fs::read(root.join(".innen").join("machine.json")) {
        Ok(bytes) => bytes,
        Err(_) => return Layer::default(),
    };
    parse_machine_bytes(&bytes)
}

/// Parse `INNEN_REBUILD_ON_OPEN` values (`true`/`false`, case-insensitive).
pub(super) fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

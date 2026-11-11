//! Precedence merge and the two load entry points.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::layers::{parse_env_bool, read_machine_layer, read_toml_layer, Layer};
use super::{CliOverrides, Config};
use crate::config::global::{get_global_value_with_env, global_config_path_with_env};

/// Default harvest patterns following llm-wiki "File over app" principle.
const DEFAULT_HARVEST_PATTERNS: &[&str] = &["**/*.md", "**/*.json", "**/*.jsonl", "**/*.txt"];

/// Read global user config values (root, format, rebuild_on_open only — inbox_path is project-fixed).
fn read_global_layer(env: Option<&HashMap<String, String>>) -> Layer {
    let mut layer = Layer::default();
    let global_path = match global_config_path_with_env(env) {
        Ok(p) => p,
        Err(_) => return layer,
    };
    let bytes = match std::fs::read(&global_path) {
        Ok(b) => b,
        Err(_) => return layer,
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let val: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return layer,
    };
    let Some(obj) = val.as_object() else {
        return layer;
    };
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
    // inbox_path intentionally NOT read from global config — it's project-fixed via machine.json
    if let Some(arr) = obj.get("harvest_patterns").and_then(|v| v.as_array()) {
        let patterns: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !patterns.is_empty() {
            layer.harvest_patterns = Some(patterns);
        }
    }
    if let Some(b) = obj.get("rebuild_on_open").and_then(|v| v.as_bool()) {
        layer.rebuild_on_open = Some(b);
    }
    layer
}

/// Merge layers in precedence order and collect machine-over-toml warnings.
fn merge(
    root_arg: &Path,
    toml: &Layer,
    machine: &Layer,
    global: &Layer,
    env_root: Option<PathBuf>,
    env_format: Option<String>,
    env_rebuild: Option<bool>,
    cli: &CliOverrides,
) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    for key in [
        "root",
        "format",
        "inbox_path",
        "harvest_patterns",
        "rebuild_on_open",
    ] {
        let both = match key {
            "root" => toml.root.is_some() && machine.root.is_some(),
            "format" => toml.format.is_some() && machine.format.is_some(),
            "inbox_path" => toml.inbox_path.is_some() && machine.inbox_path.is_some(),
            "harvest_patterns" => {
                toml.harvest_patterns.is_some() && machine.harvest_patterns.is_some()
            }
            _ => toml.rebuild_on_open.is_some() && machine.rebuild_on_open.is_some(),
        };
        if both {
            warnings.push(format!("warning: machine.json overrides innen.toml: {key}"));
        }
    }

    let mut cfg = Config {
        root: root_arg.to_path_buf(),
        format: "human".to_string(),
        inbox_path: PathBuf::from("00-inbox/harvest"),
        harvest_patterns: DEFAULT_HARVEST_PATTERNS.iter().map(|s| s.to_string()).collect(),
        rebuild_on_open: false,
    };
    if let Some(v) = &toml.root {
        cfg.root = v.clone();
    }
    if let Some(v) = &toml.format {
        cfg.format = v.clone();
    }
    if let Some(v) = &toml.inbox_path {
        cfg.inbox_path = v.clone();
    }
    if let Some(v) = &toml.harvest_patterns {
        cfg.harvest_patterns = v.clone();
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
    if let Some(v) = &machine.inbox_path {
        cfg.inbox_path = v.clone();
    }
    if let Some(v) = &machine.harvest_patterns {
        cfg.harvest_patterns = v.clone();
    }
    if let Some(v) = machine.rebuild_on_open {
        cfg.rebuild_on_open = v;
    }
    if let Some(v) = &global.root {
        cfg.root = v.clone();
    }
    if let Some(v) = &global.format {
        cfg.format = v.clone();
    }
    // inbox_path NOT from global — project-fixed via machine.json
    if let Some(v) = &global.harvest_patterns {
        cfg.harvest_patterns = v.clone();
    }
    if let Some(v) = global.rebuild_on_open {
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
    // inbox_path NOT from CLI — project-fixed via machine.json
    if let Some(v) = &cli.harvest_patterns {
        cfg.harvest_patterns = v.clone();
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
    let global = read_global_layer(None);
    merge(
        root,
        &toml,
        &machine,
        &global,
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
    let toml = read_toml_layer(root);
    let machine = read_machine_layer(root);
    let global = read_global_layer(Some(env));
    merge(
        root,
        &toml,
        &machine,
        &global,
        env_root,
        env_format,
        env_rebuild,
        cli,
    )
}

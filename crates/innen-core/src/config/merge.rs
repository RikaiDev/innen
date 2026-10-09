//! Precedence merge and the two load entry points.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::layers::{parse_env_bool, read_machine_layer, read_toml_layer, Layer};
use super::{CliOverrides, Config};

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

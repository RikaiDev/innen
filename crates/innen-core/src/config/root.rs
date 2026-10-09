//! KB root resolution and the machine.json writer.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::global::global_config_path_with_env;
use super::ConfigSetError;

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

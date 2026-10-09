//! The global (per-user) config file: locate, read and write.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::RootResolutionError;

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

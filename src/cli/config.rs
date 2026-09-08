#[derive(clap::Subcommand)]
pub(super) enum ConfigOp {
    /// Print one resolved config value as JSON.
    Get {
        /// Key: root | format | rebuild_on_open.
        key: String,
        /// Read from user-level global config (~/.config/innen/config.json).
        #[arg(long)]
        global: bool,
    },
    /// Persist one key into .innen/machine.json (or global config with --global).
    Set {
        /// Key: root | format | rebuild_on_open.
        key: String,
        /// Value to store.
        value: String,
        /// Persist to user-level global config (~/.config/innen/config.json).
        #[arg(long)]
        global: bool,
    },
}

use super::util::{escape_tsv_field, is_human};

pub(super) fn cmd_config_get(root: &std::path::Path, format: &str, key: &str) -> i32 {
    // Resolved config (env > machine.json > innen.toml > builtin). The global
    // --format rendering flag is intentionally NOT a config override here:
    // `config get format` reports the persisted value.
    let (cfg, warnings) =
        innen_core::config::load(root, &innen_core::config::CliOverrides::default());
    for w in &warnings {
        eprintln!("{w}");
    }
    let Some(value) = config_value(&cfg, key) else {
        eprintln!("error: unknown config key: {key} (root|format|rebuild_on_open)");
        return 1;
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&value));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": value}))
                .expect("config output serializes")
        );
    }
    0
}

pub(super) fn cmd_config_set_global(format: &str, key: &str, value: &str) -> i32 {
    let echo = match innen_core::config::set_global_value(key, value) {
        Ok(echo) => echo,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&echo));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
}

// Resolve the `rclone` binary via PATH lookup (CLI-only; unit tests
// inject the fixture path directly into `Rclone { bin }`).

pub(super) fn config_value(cfg: &innen_core::config::Config, key: &str) -> Option<String> {
    match key {
        "root" => Some(cfg.root.to_string_lossy().into_owned()),
        "format" => Some(cfg.format.clone()),
        "rebuild_on_open" => Some(cfg.rebuild_on_open.to_string()),
        _ => None,
    }
}

/// Persist a supported machine configuration value through the core validator.
pub(super) fn cmd_config_set(root: &std::path::Path, format: &str, key: &str, value: &str) -> i32 {
    // Thin call: validation + machine.json persistence lives in innen-core::config.
    let echo = match innen_core::config::set_machine_value(root, key, value) {
        Ok(echo) => echo,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&echo));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
}

pub(super) fn cmd_config_get_global(format: &str, key: &str) -> i32 {
    let value = match innen_core::config::get_global_value(key) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!("error: key '{key}' is not set in global config");
            return 1;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&value));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": value}))
                .expect("config output serializes")
        );
    }
    0
}

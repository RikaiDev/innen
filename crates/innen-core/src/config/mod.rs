//! Config precedence + minimal TOML reader.
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

mod global;
mod layers;
mod merge;
mod root;

#[cfg(test)]
mod tests;

pub use global::{
    get_global_value, get_global_value_with_env, global_config_dir, global_config_dir_with_env,
    global_config_path, global_config_path_with_env, set_global_value, set_global_value_with_env,
    ConfigSetError,
};
pub use layers::{CliOverrides, Config};
pub use merge::{load, load_with_env};
pub use root::{resolve_root, resolve_root_with_env, set_machine_value, RootResolutionError};

//! Exact-identity discovery; no full-text search or ingestion side effects.
//!
//! OpenCode stores sessions in SQLite and its schema changed name in v2; that
//! reader lives in [`super::opencode_db`] and is re-exported here so callers
//! reach session storage through one module.
pub use super::opencode_db::load_database;
pub(crate) use super::opencode_db::{
    database_child_sessions, load_database_session, session_table, sqlite,
};

/// Whether an Antigravity conversation database still holds dialogue.
///
/// Antigravity rotates a trajectory out of its database once the dialogue is no
/// longer needed, leaving a metadata-only stub. Such a session has no dialogue
/// anywhere, which is different from a transcript that cannot be read: the
/// first is extractable (with nothing to extract), the second is not.
pub(crate) fn antigravity_dialogue_is_absent(path: &Path) -> Option<bool> {
    let rows = sqlite(path, "SELECT json_object('n', count(*)) FROM steps").ok()?;
    let count = rows.first()?.get("n")?.as_i64()?;
    Some(count == 0)
}

fn err(path: &Path, e: impl std::fmt::Display) -> ReadError {
    ReadError(format!("read {}: {e}", path.display()))
}
use super::ReadError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Antigravity,
    Codex,
    Claude,
    Gemini,
    Mcode,
    Opencode,
    Grok,
    Copilot,
    Cursor,
    Vscode,
    Qwen,
}

impl Source {
    #[allow(non_upper_case_globals)]
    pub const Agy: Source = Source::Antigravity;
    pub const AGY: Source = Source::Antigravity;

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Mcode => "mcode",
            Self::Opencode => "opencode",
            Self::Grok => "grok",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::Vscode => "vscode",
            Self::Qwen => "qwen",
        }
    }

    pub fn parse(name: &str) -> Result<Self, ReadError> {
        match name {
            "agy" | "antigravity" => Ok(Self::Antigravity),
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "mcode" | "minimax" => Ok(Self::Mcode),
            "opencode" => Ok(Self::Opencode),
            "grok" => Ok(Self::Grok),
            "copilot" => Ok(Self::Copilot),
            "cursor" => Ok(Self::Cursor),
            "vscode" => Ok(Self::Vscode),
            "qwen" => Ok(Self::Qwen),
            _ => Err(ReadError(format!("unknown source: {name}"))),
        }
    }
}

pub struct Located {
    pub source: Source,
    pub path: PathBuf,
}

pub fn validate_id(id: &str) -> Result<String, ReadError> {
    let uuid = id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if uuid {
        return Ok(id.to_ascii_lowercase());
    }
    if (id.starts_with("ses_") || id.starts_with("mvs_"))
        && (5..=128).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Ok(id.into());
    }
    Err(ReadError(
        "invalid UUID or native session ID: expected a full UUID or a native session ID (opencode ses_..., mcode mvs_...)".into(),
    ))
}

/// Resolves the Mcode runtime-state database under a store root: the file
/// itself, or one of the two conventional layouts below it.
pub(crate) fn mcode_runtime_db(root: &Path) -> PathBuf {
    if root.is_file() {
        return root.to_path_buf();
    }
    let direct = root.join("runtime-state.sqlite");
    if direct.exists() {
        return direct;
    }
    root.join("sqlite").join("runtime-state.sqlite")
}

/// Mcode transcripts live in dated directories whose names end with
/// `-session_<base64(session_id)>`; the suffix is deterministic, so a store
/// can be searched without opening the runtime database.
pub(crate) fn mcode_session_dir_suffix(id: &str) -> Option<String> {
    id.strip_prefix("mvs_").filter(|rest| !rest.is_empty())?;
    Some(format!("-session_{}", base64_nopad(id.as_bytes())))
}

fn base64_nopad(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let packed = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(packed >> 18) as usize & 63] as char);
        out.push(TABLE[(packed >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(packed >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[packed as usize & 63] as char);
        }
    }
    out
}

pub(crate) fn default_stores() -> Result<Vec<(Source, PathBuf)>, ReadError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| ReadError("HOME unavailable; use --source and --source-root".into()))?;
    let home = PathBuf::from(home);
    let mut stores = vec![
        (Source::Codex, home.join(".codex/sessions")),
        (Source::Codex, home.join(".codex/archived_sessions")),
        (Source::Claude, home.join(".claude/projects")),
        (Source::Qwen, home.join(".qwen/projects")),
        (Source::Gemini, home.join(".gemini/tmp")),
        (Source::Grok, home.join(".grok/sessions")),
        (Source::Copilot, home.join(".copilot/session-state")),
        (Source::Cursor, home.join(".cursor/projects")),
        (
            Source::Opencode,
            home.join(".local/share/opencode/opencode.db"),
        ),
        (
            Source::Mcode,
            home.join(".minimax/v2/sqlite/runtime-state.sqlite"),
        ),
    ];
    for name in ["antigravity-cli", "antigravity", "antigravity-ide"] {
        stores.push((
            Source::Antigravity,
            home.join(".gemini").join(name).join("brain"),
        ));
    }
    for app in ["Code", "Code - Insiders", "VSCodium"] {
        stores.push((
            Source::Vscode,
            home.join("Library/Application Support")
                .join(app)
                .join("User/workspaceStorage"),
        ));
        stores.push((
            Source::Vscode,
            home.join(".config").join(app).join("User/workspaceStorage"),
        ));
    }
    Ok(stores)
}

mod document;
mod locate;

pub use document::load_document;
pub use locate::locate;

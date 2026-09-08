use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) const GRAPH_LIMIT: u64 = 64 * 1024 * 1024;
pub(super) const CACHE_LIMIT: u64 = 128 * 1024 * 1024;
pub(super) const CACHE_SCHEMA: u32 = 5;

#[derive(Debug, thiserror::Error)]
#[error("trace: {0}")]
pub struct Error(pub(super) String);

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub q: String,
    pub limit: usize,
    pub max_sources: usize,
    pub max_bytes: u64,
    pub max_records: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub seed: Option<String>,
    pub role: Option<String>,
    pub source_root: Option<PathBuf>,
    pub refresh: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            q: String::new(),
            limit: 5,
            max_sources: 8,
            max_bytes: 32 * 1024 * 1024,
            max_records: 50_000,
            max_nodes: 128,
            max_depth: 3,
            seed: None,
            role: None,
            source_root: None,
            refresh: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Locator {
    File {
        path: String,
    },
    Conversation {
        source: String,
        session_id: String,
        #[serde(default)]
        source_root: Option<String>,
    },
    Url {
        url: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LexDoc {
    pub(super) id: String,
    pub(super) tf: BTreeMap<String, u32>,
    pub(super) len: usize,
}

pub(super) fn valid_session_id(s: &str) -> bool {
    (s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        }))
        || (s.starts_with("ses_")
            && (5..=128).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

//! The report shapes. These are stable output read by other tools: fields may
//! not be added, renamed or removed, so a new case goes through an existing one.

/// Dry-run report for one tap.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TapReport {
    pub id: String,
    pub new_files: Vec<String>,
    pub skipped: Vec<String>,
    /// `missing` | `current` | `legacy_count_rebuilt` | `unreadable_rebuilt`.
    /// A rebuilt cursor means the inbox was re-listed in full, so the backlog
    /// below is authoritative rather than a continuation of an old count.
    pub cursor: String,
}

/// Dry-run report over all taps (currently exactly one, `harvest-dir`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarvestReport {
    pub taps: Vec<TapReport>,
}

/// A file skipped for credentials (never appended).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Skipped {
    pub path: String,
    pub pattern: String,
    pub preview: String,
}

/// Result of [`ingest::run`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IngestReport {
    pub added: u64,
    pub skipped: Vec<Skipped>,
}

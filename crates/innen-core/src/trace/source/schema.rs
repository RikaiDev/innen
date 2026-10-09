use super::super::metadata::Reference;
use super::super::types::{Error, LexDoc, Locator};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::trace) struct Version {
    pub(in crate::trace) bytes: u64,
    pub(in crate::trace) modified_ns: u128,
}
pub(in crate::trace) fn version(path: &Path) -> Result<Version, Error> {
    let m = fs::metadata(path)?;
    if !m.is_file() {
        return Err(Error(format!(
            "{} is not a regular source file",
            path.display()
        )));
    }
    Ok(Version {
        bytes: m.len(),
        modified_ns: m
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error(e.to_string()))?
            .as_nanos(),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::trace) struct Passage {
    pub(in crate::trace) line: usize,
    pub(in crate::trace) end_line: usize,
    pub(in crate::trace) role: String,
    pub(in crate::trace) timestamp: Option<String>,
    pub(in crate::trace) text: String,
    pub(in crate::trace) hash: String,
    pub(in crate::trace) start_byte: Option<u64>,
    pub(in crate::trace) end_byte: Option<u64>,
    #[serde(default)]
    pub(in crate::trace) source_pointer: Option<String>,
    #[serde(default)]
    pub(in crate::trace) scalar_sha256: Option<String>,
    #[serde(default)]
    pub(in crate::trace) ordinal: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::trace) struct SourceCache {
    pub(in crate::trace) schema: u32,
    pub(in crate::trace) locator: Locator,
    pub(in crate::trace) source_path: String,
    pub(in crate::trace) version: Version,
    pub(in crate::trace) complete: bool,
    pub(in crate::trace) projection_incomplete: bool,
    pub(in crate::trace) max_bytes: u64,
    pub(in crate::trace) max_records: usize,
    pub(in crate::trace) warnings: Vec<String>,
    pub(in crate::trace) source_sha256: String,
    pub(in crate::trace) records_scanned: usize,
    pub(in crate::trace) passages: Vec<Passage>,
    pub(in crate::trace) docs: Vec<LexDoc>,
    #[serde(default)]
    pub(in crate::trace) references: Vec<Reference>,
}

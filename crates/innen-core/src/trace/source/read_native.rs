use super::super::types::{Error, Options};
use super::schema::{version, Passage};
use super::ReadOutcome;
use crate::conversation::{self, scan};
use crate::ids::sha256_hex;
use std::path::Path;

fn native_passage(record: conversation::Record) -> Option<Passage> {
    let text = record.event.get("content")?.as_str()?.to_string();
    Some(Passage {
        line: record.line,
        end_line: record.line,
        role: record.event["role"].as_str().unwrap_or("unknown").into(),
        timestamp: record.event["timestamp"].as_str().map(str::to_owned),
        text,
        hash: sha256_hex(&serde_json::to_vec(&record.event).expect("record serializes")),
        start_byte: None,
        end_byte: None,
        source_pointer: record.event["source_pointer"].as_str().map(str::to_owned),
        scalar_sha256: record.event["source_scalar_sha256"]
            .as_str()
            .map(str::to_owned),
        ordinal: record.event["ordinal"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok()),
    })
}

pub(super) fn read_native(
    source: &str,
    id: &str,
    source_root: Option<&str>,
    opts: &Options,
    remaining: u64,
) -> Result<ReadOutcome, Error> {
    let result = scan::scan(
        source_root.map(Path::new),
        source,
        id,
        &scan::ScanLimits {
            max_bytes: remaining,
            max_records: opts.max_records,
        },
    )
    .map_err(|e| Error(e.to_string()))?;
    let version = version(&result.source_path)?;
    let expected = format!(
        "size={};mtime_unix_ns={}",
        version.bytes, version.modified_ns
    );
    if expected != result.source_version {
        return Err(Error("native source changed after scan".into()));
    }
    Ok(ReadOutcome {
        path: result.source_path,
        version,
        passages: result
            .records
            .into_iter()
            .filter_map(native_passage)
            .collect(),
        complete: result.complete && !result.projection_incomplete,
        projection_incomplete: result.projection_incomplete,
        warnings: result.warnings,
        hash: result.source_sha256,
        scanned: result.records_scanned,
        bytes: result.bytes_read,
        references: Vec::new(),
    })
}

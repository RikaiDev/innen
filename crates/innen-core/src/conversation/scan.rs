//! One-pass, bounded scanning of a native conversation source.

use super::{projection, sources, ReadError, Record, Source};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_bytes: u64,
    pub max_records: usize,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub session_id: String,
    pub source: Source,
    pub source_path: PathBuf,
    pub records: Vec<Record>,
    /// Source bytes logically consumed by the scanner. This excludes locator and
    /// metadata I/O as well as any `BufReader` read-ahead beyond consumed bytes.
    pub bytes_read: u64,
    pub records_scanned: usize,
    pub complete: bool,
    /// True when one or more complete source records could not be projected.
    pub projection_incomplete: bool,
    pub warnings: Vec<String>,
    pub source_sha256: String,
    pub source_version: String,
}

/// Resolve the native session exactly once, then scan its dialogue projection.
pub fn scan(
    source_root: Option<&Path>,
    source: &str,
    id: &str,
    limits: &ScanLimits,
) -> Result<ScanResult, ReadError> {
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    scan_located(located.source, located.path, id, limits)
}

fn scan_located(
    source: Source,
    path: PathBuf,
    id: String,
    limits: &ScanLimits,
) -> Result<ScanResult, ReadError> {
    let file = File::open(&path).map_err(|error| io_error(&path, error))?;
    let metadata = file.metadata().map_err(|error| io_error(&path, error))?;
    let version = source_version(&path, &metadata)?;
    let mut result = ScanResult {
        session_id: id,
        source,
        source_path: path.clone(),
        records: Vec::new(),
        bytes_read: 0,
        records_scanned: 0,
        complete: false,
        projection_incomplete: false,
        warnings: Vec::new(),
        source_sha256: String::new(),
        source_version: version,
    };
    let mut hasher = Sha256::new();

    if source == Source::Opencode {
        return Err(ReadError(
            "bounded conversation scan unsupported for OpenCode: database byte reads cannot be measured"
                .into(),
        ));
    }
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        scan_document(file, &metadata, limits, &mut result, &mut hasher)?;
    } else {
        scan_jsonl(file, limits, &mut result, &mut hasher)?;
    }
    let final_metadata = std::fs::metadata(&path).map_err(|error| io_error(&path, error))?;
    if source_version(&path, &final_metadata)? != result.source_version {
        return Err(ReadError(format!(
            "conversation source changed while scanning: {}",
            path.display()
        )));
    }
    result.source_sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !result.complete {
        result.warnings.push(format!(
            "source_sha256 covers only the {} source bytes consumed by this partial scan",
            result.bytes_read
        ));
    }
    Ok(result)
}

fn scan_jsonl(
    file: File,
    limits: &ScanLimits,
    result: &mut ScanResult,
    hasher: &mut Sha256,
) -> Result<(), ReadError> {
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut oversized = false;
    let mut line_number = 0usize;

    loop {
        if result.records_scanned == limits.max_records {
            result.complete = reader
                .fill_buf()
                .map_err(|error| io_error(&result.source_path, error))?
                .is_empty();
            if !result.complete {
                result.warnings.push(format!(
                    "scan stopped at max_records={}",
                    limits.max_records
                ));
            }
            return Ok(());
        }
        if result.bytes_read == limits.max_bytes {
            let eof = reader
                .fill_buf()
                .map_err(|error| io_error(&result.source_path, error))?
                .is_empty();
            if eof && (!line.is_empty() || oversized) {
                line_number += 1;
                accept_line(result, line_number, &line, oversized)?;
                line.clear();
            }
            result.complete = eof;
            if !result.complete {
                result
                    .warnings
                    .push(format!("scan stopped at max_bytes={}", limits.max_bytes));
            }
            return Ok(());
        }

        let available = reader
            .fill_buf()
            .map_err(|error| io_error(&result.source_path, error))?;
        if available.is_empty() {
            if !line.is_empty() || oversized {
                line_number += 1;
                accept_line(result, line_number, &line, oversized)?;
            }
            result.complete = true;
            return Ok(());
        }

        let remaining = usize::try_from(limits.max_bytes - result.bytes_read).unwrap_or(usize::MAX);
        let visible = &available[..available.len().min(remaining)];
        let newline = visible.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(visible.len(), |position| position + 1);
        let chunk = &visible[..take];
        if !oversized {
            if line.len().saturating_add(chunk.len()) > MAX_LINE_BYTES {
                line.clear();
                oversized = true;
            } else {
                line.extend_from_slice(chunk);
            }
        }
        hasher.update(chunk);
        result.bytes_read += take as u64;
        reader.consume(take);

        if newline.is_some() {
            line_number += 1;
            accept_line(result, line_number, &line, oversized)?;
            line.clear();
            oversized = false;
        }
    }
}

fn accept_line(
    result: &mut ScanResult,
    line_number: usize,
    bytes: &[u8],
    oversized: bool,
) -> Result<(), ReadError> {
    result.records_scanned += 1;
    if oversized {
        result.projection_incomplete = true;
        result.warnings.push(format!(
            "omitted oversized source line {line_number} from dialogue projection; line exceeds {MAX_LINE_BYTES} bytes"
        ));
        return Ok(());
    }
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ReadError(format!(
            "invalid UTF-8 in transcript at {}:{}",
            result.source_path.display(),
            line_number
        ))
    })?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let event: Value = serde_json::from_str(text).map_err(|error| {
        ReadError(format!(
            "invalid transcript JSON at {}:{}: {error}",
            result.source_path.display(),
            line_number
        ))
    })?;
    accept_event(result, line_number, event, false)
}

fn accept_event(
    result: &mut ScanResult,
    line_number: usize,
    event: Value,
    count_record: bool,
) -> Result<(), ReadError> {
    if !event.is_object() {
        return Err(ReadError(format!(
            "unsupported transcript event at {}:{}: expected object",
            result.source_path.display(),
            line_number
        )));
    }
    if count_record {
        result.records_scanned += 1;
    }
    if let Some(event) = projection::dialogue(result.source, event)? {
        result.records.push(Record {
            line: line_number,
            event,
        });
    }
    Ok(())
}

fn scan_document(
    mut file: File,
    metadata: &Metadata,
    limits: &ScanLimits,
    result: &mut ScanResult,
    hasher: &mut Sha256,
) -> Result<(), ReadError> {
    let allowed = metadata.len().min(limits.max_bytes);
    let capacity = usize::try_from(allowed).map_err(|_| {
        ReadError(format!(
            "source byte limit is too large for this platform: {}",
            limits.max_bytes
        ))
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.by_ref()
        .take(allowed)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(&result.source_path, error))?;
    result.bytes_read = bytes.len() as u64;
    hasher.update(&bytes);
    if result.bytes_read < allowed {
        return Err(ReadError(format!(
            "conversation source changed while scanning: {}",
            result.source_path.display()
        )));
    }
    if allowed < metadata.len() {
        result
            .warnings
            .push(format!("scan stopped at max_bytes={}", limits.max_bytes));
        return Ok(());
    }
    let mut changed_byte = [0u8; 1];
    if file
        .read(&mut changed_byte)
        .map_err(|error| io_error(&result.source_path, error))?
        != 0
    {
        return Err(ReadError(format!(
            "conversation source changed while scanning: {}",
            result.source_path.display()
        )));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ReadError(format!(
            "invalid UTF-8 in transcript at {}",
            result.source_path.display()
        ))
    })?;
    let document: Value = serde_json::from_str(text).map_err(|error| {
        ReadError(format!(
            "invalid transcript JSON at {}: {error}",
            result.source_path.display()
        ))
    })?;
    if !document.is_object() {
        return Err(ReadError(format!(
            "unsupported transcript document at {}: expected object",
            result.source_path.display()
        )));
    }
    let key = match result.source {
        Source::Gemini => "messages",
        Source::Vscode => "requests",
        _ => {
            return Err(ReadError(format!(
                "unsupported transcript document source: {}",
                result.source.as_str()
            )))
        }
    };
    let events = document.get(key).and_then(Value::as_array).ok_or_else(|| {
        ReadError(format!(
            "{} document missing {key} array",
            if result.source == Source::Gemini {
                "Gemini"
            } else {
                "VS Code"
            }
        ))
    })?;
    for (index, event) in events.iter().enumerate() {
        if result.records_scanned == limits.max_records {
            result.warnings.push(format!(
                "scan stopped at max_records={}",
                limits.max_records
            ));
            return Ok(());
        }
        accept_event(result, index + 1, event.clone(), true)?;
    }
    result.complete = true;
    Ok(())
}

fn source_version(path: &Path, metadata: &Metadata) -> Result<String, ReadError> {
    let modified = metadata
        .modified()
        .map_err(|error| io_error(path, error))?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ReadError(format!("read source metadata {}: {error}", path.display())))?
        .as_nanos();
    Ok(format!("size={};mtime_unix_ns={modified}", metadata.len()))
}

fn io_error(path: &Path, error: std::io::Error) -> ReadError {
    ReadError(format!("read {}: {error}", path.display()))
}

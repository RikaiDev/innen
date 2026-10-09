use super::super::metadata;
use super::super::types::{Error, Options};
use super::schema::{version, Passage, Version};
use super::ReadOutcome;
use crate::ids::sha256_hex;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(in crate::trace) struct PassageScan {
    pub(in crate::trace) passages: Vec<Passage>,
    pub(in crate::trace) records: usize,
    pub(in crate::trace) truncated: bool,
}

pub(in crate::trace) fn file_passages(text: &str, limit: usize) -> PassageScan {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut role = "source".to_string();
    let mut records = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if i >= limit {
            return PassageScan {
                passages: out,
                records,
                truncated: true,
            };
        }
        records = i + 1;
        let heading = line
            .trim()
            .trim_matches('#')
            .trim()
            .trim_matches('*')
            .trim()
            .to_lowercase();
        if matches!(heading.as_str(), "user" | "assistant" | "使用者" | "助理") {
            role = if matches!(heading.as_str(), "user" | "使用者") {
                "user"
            } else {
                "assistant"
            }
            .into();
        }
        // Fixed byte-bounded chunks on real UTF-8 boundaries, including long JSON lines.
        let mut start = 0;
        while start < line.len() {
            let mut end = (start + 4096).min(line.len());
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            let part = &line[start..end];
            if !part.trim().is_empty() {
                if out.len() >= limit {
                    return PassageScan {
                        passages: out,
                        records,
                        truncated: true,
                    };
                }
                out.push(Passage {
                    line: i + 1,
                    end_line: i + 1,
                    role: role.clone(),
                    timestamp: None,
                    text: part.into(),
                    hash: sha256_hex(part.as_bytes()),
                    start_byte: Some((offset + start) as u64),
                    end_byte: Some((offset + end) as u64),
                    source_pointer: None,
                    scalar_sha256: None,
                    ordinal: None,
                });
            }
            start = end;
        }
        offset += line.len();
    }
    PassageScan {
        passages: out,
        records,
        truncated: false,
    }
}

pub(super) fn file_preflight(path: &Path) -> Result<Version, Error> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    // The locator is already an explicit provenance edge. Keep the format
    // gate conservative, but accept UTF-8 logs and extensionless text files;
    // `read_file` remains the final binary/UTF-8 validation boundary.
    if !(ext.is_empty()
        || matches!(
            ext.as_str(),
            "md" | "txt" | "json" | "jsonl" | "yaml" | "yml" | "log"
        ))
    {
        return Err(Error(format!(
            "unsupported source type .{ext}: {}",
            path.display()
        )));
    }
    version(path)
}

pub(super) fn read_file(
    path: &Path,
    before: Version,
    opts: &Options,
    remaining: u64,
) -> Result<ReadOutcome, Error> {
    let mut bytes = Vec::new();
    File::open(path)?.take(remaining).read_to_end(&mut bytes)?;
    let read = bytes.len() as u64;
    let complete = read == before.bytes;
    let mut warnings = Vec::new();
    if !complete {
        warnings.push("source_byte_budget_truncated".into());
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(e) if !complete && e.error_len().is_none() => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).map_err(|e| Error(e.to_string()))?
        }
        Err(e) => return Err(Error(format!("source UTF-8: {e}"))),
    };
    let scan = file_passages(text, opts.max_records);
    let metadata = if complete && !scan.truncated {
        metadata::references(text)
    } else {
        metadata::Metadata::default()
    };
    let projection_incomplete = scan.truncated || metadata.truncated;
    if scan.truncated {
        warnings.push("passage_budget_truncated".into());
    }
    if metadata.truncated {
        warnings.push("metadata_reference_budget_truncated".into());
    }
    if version(path)? != before {
        return Err(Error("source changed during read".into()));
    }
    Ok(ReadOutcome {
        path: path.to_path_buf(),
        version: before,
        passages: scan.passages,
        complete: complete && !projection_incomplete,
        projection_incomplete,
        warnings,
        hash: sha256_hex(&bytes),
        scanned: scan.records,
        bytes: read,
        references: metadata.references,
    })
}

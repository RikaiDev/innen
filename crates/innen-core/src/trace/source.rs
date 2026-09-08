use super::cache::{atomic, bounded, cache_root};
use super::lexical::lex;
use super::metadata::{self, Reference};
use super::types::{valid_session_id, Error, Locator, Options, CACHE_LIMIT, CACHE_SCHEMA};
use crate::conversation::{self, scan, Source};
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Version {
    pub(super) bytes: u64,
    pub(super) modified_ns: u128,
}
pub(super) fn version(path: &Path) -> Result<Version, Error> {
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
pub(super) struct Passage {
    pub(super) line: usize,
    pub(super) end_line: usize,
    pub(super) role: String,
    pub(super) timestamp: Option<String>,
    pub(super) text: String,
    pub(super) hash: String,
    pub(super) start_byte: Option<u64>,
    pub(super) end_byte: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SourceCache {
    pub(super) schema: u32,
    pub(super) locator: Locator,
    pub(super) source_path: String,
    pub(super) version: Version,
    pub(super) complete: bool,
    pub(super) projection_incomplete: bool,
    pub(super) max_bytes: u64,
    pub(super) max_records: usize,
    pub(super) warnings: Vec<String>,
    pub(super) source_sha256: String,
    pub(super) records_scanned: usize,
    pub(super) passages: Vec<Passage>,
    pub(super) docs: Vec<super::types::LexDoc>,
    #[serde(default)]
    pub(super) references: Vec<Reference>,
}

pub(super) fn resolve_locator(
    loc: &Locator,
    opts: &Options,
    catalog: &mut BTreeMap<String, Vec<conversation::Candidate>>,
    catalog_count: &mut usize,
) -> Result<Locator, Error> {
    if let Locator::Conversation {
        source,
        session_id,
        source_root,
    } = loc
    {
        let effective_root = opts
            .source_root
            .as_ref()
            .map(|p| p.display().to_string())
            .or_else(|| source_root.clone());
        if valid_session_id(session_id) {
            return Ok(Locator::Conversation {
                source: source.clone(),
                session_id: if session_id.len() == 36 {
                    session_id.to_ascii_lowercase()
                } else {
                    session_id.clone()
                },
                source_root: effective_root,
            });
        }
        let compact = session_id.replace('-', "");
        if compact.len() < 8 || !compact.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error(format!(
                "unresolved conversation identity {source}:{session_id}"
            )));
        }
        let catalog_key = serde_json::to_string(&(source, &effective_root))?;
        if !catalog.contains_key(&catalog_key) {
            let provider = Source::parse(source).map_err(|e| Error(e.to_string()))?;
            let rows = conversation::find_all_candidates(
                Some(provider),
                effective_root.as_deref().map(Path::new),
            )
            .map_err(|e| Error(e.to_string()))?;
            *catalog_count += rows.len();
            catalog.insert(catalog_key.clone(), rows);
        }
        let matches: Vec<_> = catalog[&catalog_key]
            .iter()
            .filter(|c| c.id.replace('-', "").starts_with(&compact))
            .collect();
        if matches.len() != 1 {
            return Err(Error(format!(
                "short conversation identity {source}:{session_id} resolves to {} candidates",
                matches.len()
            )));
        }
        return Ok(Locator::Conversation {
            source: source.clone(),
            session_id: matches[0].id.clone(),
            source_root: effective_root,
        });
    }
    Ok(loc.clone())
}

pub(super) struct PassageScan {
    pub(super) passages: Vec<Passage>,
    pub(super) records: usize,
    pub(super) truncated: bool,
}

pub(super) fn file_passages(text: &str, limit: usize) -> PassageScan {
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

fn load_cached(
    root: &Path,
    loc: &Locator,
    opts: &Options,
    remaining: u64,
) -> Result<Option<(SourceCache, String)>, Error> {
    let alias = sha256_hex(&serde_json::to_vec(loc)?);
    let dir = cache_root(root).join("sources");
    let head = match bounded(&dir.join(format!("{alias}.ref")), 128) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let key = match String::from_utf8(head) {
        Ok(key) if valid_key(&key) => key,
        _ => return Ok(None),
    };
    let bytes = match bounded(&dir.join(format!("{key}.json")), CACHE_LIMIT) {
        Ok(bytes) if sha256_hex(&bytes) == key => bytes,
        _ => return Ok(None),
    };
    let cached: SourceCache = match serde_json::from_slice(&bytes) {
        Ok(cached) => cached,
        Err(_) => return Ok(None),
    };
    let compatible = cached.complete
        || (cached.max_bytes == remaining && cached.max_records == opts.max_records);
    if cached.schema != CACHE_SCHEMA || cached.locator != *loc || !compatible {
        return Ok(None);
    }
    Ok(version(Path::new(&cached.source_path))
        .is_ok_and(|v| v == cached.version)
        .then_some((cached, key)))
}

struct ReadOutcome {
    path: PathBuf,
    version: Version,
    passages: Vec<Passage>,
    complete: bool,
    projection_incomplete: bool,
    warnings: Vec<String>,
    hash: String,
    scanned: usize,
    bytes: u64,
    references: Vec<Reference>,
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub(super) struct SourceFailure {
    pub(super) error: Error,
    /// None means a failed scanner may have consumed up to its full allowance.
    pub(super) consumed: Option<u64>,
}

fn file_preflight(path: &Path) -> Result<Version, Error> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(
        ext.as_str(),
        "md" | "txt" | "json" | "jsonl" | "yaml" | "yml"
    ) {
        return Err(Error(format!(
            "unsupported source type .{ext}: {}",
            path.display()
        )));
    }
    version(path)
}

fn read_file(
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
    })
}

fn read_native(
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

fn persist(
    root: &Path,
    loc: &Locator,
    read: ReadOutcome,
    opts: &Options,
    remaining: u64,
) -> Result<(SourceCache, String), Error> {
    let docs = read
        .passages
        .iter()
        .enumerate()
        .map(|(i, p)| lex(i.to_string(), &p.text))
        .collect();
    let out = SourceCache {
        schema: CACHE_SCHEMA,
        locator: loc.clone(),
        source_path: read.path.display().to_string(),
        version: read.version,
        complete: read.complete,
        projection_incomplete: read.projection_incomplete,
        max_bytes: remaining,
        max_records: opts.max_records,
        warnings: read.warnings,
        source_sha256: read.hash,
        records_scanned: read.scanned,
        passages: read.passages,
        docs,
        references: read.references,
    };
    let encoded = serde_json::to_vec(&out)?;
    if encoded.len() as u64 > CACHE_LIMIT {
        return Err(Error("source cache exceeds bound".into()));
    }
    let key = sha256_hex(&encoded);
    let dir = cache_root(root).join("sources");
    atomic(&dir.join(format!("{key}.json")), &encoded)?;
    let alias = sha256_hex(&serde_json::to_vec(loc)?);
    atomic(&dir.join(format!("{alias}.ref")), key.as_bytes())?;
    Ok((out, key))
}

pub(super) fn source(
    root: &Path,
    loc: &Locator,
    opts: &Options,
    remaining: u64,
) -> Result<(SourceCache, String, bool, u64), SourceFailure> {
    if !opts.refresh {
        if let Some((cached, key)) =
            load_cached(root, loc, opts, remaining).map_err(|error| SourceFailure {
                error,
                consumed: Some(0),
            })?
        {
            return Ok((cached, key, true, 0));
        }
    }
    if remaining == 0 {
        return Err(SourceFailure {
            error: Error("source byte budget exhausted".into()),
            consumed: Some(0),
        });
    }
    let read = match loc {
        Locator::File { path } => {
            let before = file_preflight(Path::new(path)).map_err(|error| SourceFailure {
                error,
                consumed: Some(0),
            })?;
            read_file(Path::new(path), before, opts, remaining)
        }
        Locator::Conversation {
            source,
            session_id,
            source_root,
        } => read_native(source, session_id, source_root.as_deref(), opts, remaining),
        Locator::Url { url } => {
            return Err(SourceFailure {
                error: Error(format!("remote source not fetched: {url}")),
                consumed: Some(0),
            })
        }
    }
    .map_err(|error| SourceFailure {
        error,
        consumed: None,
    })?;
    let count = read.bytes;
    let (cached, key) =
        persist(root, loc, read, opts, remaining).map_err(|error| SourceFailure {
            error,
            consumed: Some(count),
        })?;
    Ok((cached, key, false, count))
}

pub(super) fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn excerpt(text: &str, query: &[String]) -> String {
    let lower = text.to_lowercase();
    let at = query
        .iter()
        .filter_map(|q| lower.find(q))
        .min()
        .unwrap_or(0)
        .min(text.len());
    let mut start = at.saturating_sub(120);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut out: String = text[start..].chars().take(600).collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + out.len() < text.len() {
        out.push('…');
    }
    out
}

use super::super::cache::{atomic, bounded, cache_root};
use super::super::lexical::lex;
use super::super::types::{Error, Locator, Options, CACHE_LIMIT, CACHE_SCHEMA};
use super::schema::{version, SourceCache};
use super::ReadOutcome;
use crate::ids::sha256_hex;
use std::path::Path;

pub(in crate::trace) fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn load_cached(
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

pub(super) fn persist(
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

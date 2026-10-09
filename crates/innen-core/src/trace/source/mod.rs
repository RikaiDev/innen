mod read_file;
mod read_native;
mod schema;
mod store;

use self::read_file::{file_preflight, read_file};
use self::read_native::read_native;
use self::store::{load_cached, persist};
use super::metadata::Reference;
use super::types::{valid_session_id, Error, Locator, Options};
use crate::conversation::{self, Source};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Reached by siblings in `trace`; `file_passages`/`PassageScan` are reached by
// `trace/tests.rs` only, so they live exactly as long as it does.
#[cfg(test)]
pub(in crate::trace) use self::read_file::{file_passages, PassageScan};
pub(in crate::trace) use self::schema::{Passage, SourceCache, Version};
pub(in crate::trace) use self::store::valid_key;

pub(in crate::trace) fn resolve_locator(
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
pub(in crate::trace) struct SourceFailure {
    pub(in crate::trace) error: Error,
    /// None means a failed scanner may have consumed up to its full allowance.
    pub(in crate::trace) consumed: Option<u64>,
}

pub(in crate::trace) fn source(
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

pub(in crate::trace) fn excerpt(text: &str, query: &[String]) -> String {
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

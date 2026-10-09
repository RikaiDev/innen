use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use super::{projection, sources, Located, Page, ReadError, Record, Source};

/// Offset counts physical rows in both views. Read one matching event ahead so
/// next_offset signals actual remaining content, not merely reaching the limit.
/// Invalid source data fails visibly with line provenance, never an empty success.
pub fn read(
    source_root: Option<&Path>,
    source: &str,
    id: &str,
    view: &str,
    offset: usize,
    limit: usize,
) -> Result<Page, ReadError> {
    if !matches!(view, "dialogue" | "events" | "index" | "context") || !(1..=100).contains(&limit) {
        return Err(ReadError(
            "expected view dialogue|events|index|context and limit 1..100".into(),
        ));
    }
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    let mut page = read_located(located, id, view, offset, limit, None)?;
    if view == "index" || (view == "context" && page.source == Source::Codex) {
        projection::link_index(&mut page);
    }
    Ok(page)
}

/// Read explicitly chosen one-based source rows as a single events page. The
/// selection is caller-owned, sorted in source order, and never relevance-ranked.
pub fn read_lines(
    source_root: Option<&Path>,
    source: &str,
    id: &str,
    lines: &[usize],
) -> Result<Page, ReadError> {
    let selected: BTreeSet<_> = lines.iter().copied().collect();
    if selected.is_empty() || selected.len() > 100 || selected.contains(&0) {
        return Err(ReadError(
            "expected 1..100 distinct positive source lines".into(),
        ));
    }
    let id = sources::validate_id(id)?;
    let located = sources::locate(source_root, source, &id)?;
    let page = read_located(
        located,
        id,
        "events",
        selected.first().unwrap() - 1,
        selected.len(),
        Some(&selected),
    )?;
    for line in &selected {
        if !page.records.iter().any(|r| r.line == *line) {
            return Err(ReadError(format!(
                "requested source line {line} is absent or blank"
            )));
        }
    }
    Ok(page)
}

/// Stream every dialogue projection of one located session in a single pass,
/// without holding the session in memory. Used by retention compaction.
pub(crate) fn stream_dialogue(
    located: &Located,
    id: &str,
    mut sink: impl FnMut(usize, Value) -> Result<(), ReadError>,
) -> Result<(), ReadError> {
    let mut emit = |index: usize, event: Value| -> Result<(), ReadError> {
        if !event.is_object() {
            return Err(ReadError(format!(
                "unsupported transcript event at {}:{}: expected object",
                located.path.display(),
                index + 1
            )));
        }
        match projection::dialogue(located.source, event)? {
            Some(projected) => sink(index + 1, projected),
            None => Ok(()),
        }
    };
    if located.source == Source::Opencode {
        let mut cursor = 0;
        loop {
            let events = sources::load_database(located, id, cursor)?;
            if events.is_empty() {
                return Ok(());
            }
            let count = events.len();
            for (index, event) in events.into_iter().enumerate() {
                emit(cursor + index, event)?;
            }
            cursor += count;
        }
    }
    let path = &located.path;
    if path.extension().is_some_and(|s| s == "json") {
        for (index, event) in sources::load_document(located)?.into_iter().enumerate() {
            emit(index, event)?;
        }
        return Ok(());
    }
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| io_error(path, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(&line).map_err(|e| {
            ReadError(format!(
                "invalid transcript JSON at {}:{}: {e}",
                path.display(),
                index + 1
            ))
        })?;
        emit(index, event)?;
    }
    Ok(())
}

fn read_located(
    located: Located,
    id: String,
    view: &str,
    offset: usize,
    limit: usize,
    selected: Option<&BTreeSet<usize>>,
) -> Result<Page, ReadError> {
    let path = located.path.clone();
    let mut page = Page {
        session_id: id.clone(),
        source: located.source,
        source_path: path.clone(),
        view: view.into(),
        offset,
        next_offset: None,
        records: Vec::new(),
        warnings: Vec::new(),
    };
    if located.source == Source::Cursor {
        page.warnings.push("Cursor exported transcripts may omit tool outputs; events cannot restore data absent from the export".into());
    }
    if located.source == Source::Opencode {
        let mut cursor = offset;
        loop {
            let events = sources::load_database(&located, &id, cursor)?;
            if events.is_empty() {
                return Ok(page);
            }
            let count = events.len();
            for (index, event) in events.into_iter().enumerate() {
                if accept(&mut page, event, cursor + index, limit, selected)? {
                    return Ok(page);
                }
            }
            cursor += count;
        }
    }
    if path.extension().is_some_and(|s| s == "json") {
        let events = sources::load_document(&located)?;
        for (index, event) in events.into_iter().enumerate().skip(offset) {
            if accept(&mut page, event, index, limit, selected)? {
                break;
            }
        }
        return Ok(page);
    }
    let file = File::open(&path).map_err(|e| io_error(&path, e))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| io_error(&path, e))?;
        if selected.is_some_and(|s| index + 1 > *s.last().unwrap()) {
            break;
        }
        if index < offset
            || line.trim().is_empty()
            || selected.is_some_and(|s| !s.contains(&(index + 1)))
        {
            continue;
        }
        let event: Value = serde_json::from_str(&line).map_err(|e| {
            ReadError(format!(
                "invalid transcript JSON at {}:{}: {e}",
                path.display(),
                index + 1
            ))
        })?;
        if accept(&mut page, event, index, limit, selected)? {
            break;
        }
    }
    Ok(page)
}

fn accept(
    page: &mut Page,
    event: Value,
    index: usize,
    limit: usize,
    selected: Option<&BTreeSet<usize>>,
) -> Result<bool, ReadError> {
    if let Some(selected) = selected {
        if !selected.contains(&(index + 1)) {
            return Ok(index + 1 > *selected.last().unwrap());
        }
    }
    if !event.is_object() {
        return Err(ReadError(format!(
            "unsupported transcript event at {}:{}: expected object",
            page.source_path.display(),
            index + 1
        )));
    }
    let event = if page.view == "events" {
        Some(event)
    } else if page.view == "context" {
        projection::context(page.source, event)?
    } else if page.view == "index" {
        projection::index(page.source, event)?
    } else {
        projection::dialogue(page.source, event)?
    };
    let Some(event) = event else { return Ok(false) };
    if page.records.len() == limit {
        page.next_offset = Some(index);
        return Ok(true);
    }
    if event
        .get("truncated_fields")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        page.warnings.push(format!("source event at record {} contains truncated fields; this reader cannot restore omitted source text", index + 1));
    }
    page.records.push(Record {
        line: index + 1,
        event,
    });
    Ok(selected.is_some_and(|s| page.records.len() == s.len()))
}

pub(super) fn io_error(path: &Path, error: std::io::Error) -> ReadError {
    ReadError(format!("read {}: {error}", path.display()))
}

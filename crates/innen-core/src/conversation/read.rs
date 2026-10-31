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
///
/// A transcript that lost one line to a partial write must not cost the whole
/// session: unparseable lines are skipped and reported through `warnings`, and
/// only a transcript that yields no event at all stays unreadable.
pub(crate) fn stream_dialogue(
    located: &Located,
    id: &str,
    warnings: &mut Vec<String>,
    mut sink: impl FnMut(usize, Value) -> Result<(), ReadError>,
) -> Result<(), ReadError> {
    let path = &located.path;
    let mut emit = |index: usize, event: Value| -> Result<(), ReadError> {
        if !event.is_object() {
            return Err(ReadError(format!(
                "unsupported transcript event at {}:{}: expected object",
                path.display(),
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
    if path.extension().is_some_and(|s| s == "json") {
        for (index, event) in sources::load_document(located)?.into_iter().enumerate() {
            emit(index, event)?;
        }
        return Ok(());
    }
    if located.source == Source::Antigravity
        && path.extension().is_some_and(|ext| ext == "db")
        && sources::antigravity_dialogue_is_absent(path) == Some(true)
    {
        // The trajectory was rotated out of the database, so no dialogue
        // exists to extract. That is an empty extraction, not a read failure.
        warnings.push(format!(
            "antigravity conversation database {} holds no dialogue",
            path.display()
        ));
        return Ok(());
    }
    let reader = open_transcript(path)?;
    let mut parsed = 0usize;
    let mut first_error: Option<(usize, String)> = None;
    for (line_number, line) in BufReader::new(reader).lines().enumerate() {
        let line = line.map_err(|e| io_error(path, e))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(event) => {
                parsed += 1;
                emit(line_number, event)?;
            }
            Err(error) => {
                let message = format!(
                    "invalid transcript JSON at {}:{}: {error}",
                    path.display(),
                    line_number + 1
                );
                if first_error.is_none() {
                    first_error = Some((line_number + 1, message));
                }
                warnings.push(format!(
                    "skipped unreadable transcript line {}:{}: {error}",
                    path.display(),
                    line_number + 1
                ));
            }
        }
    }
    if parsed == 0 {
        // A transcript that yields no event at all stays unreadable, and the
        // first unreadable line keeps its provenance.
        return Err(ReadError(match first_error {
            Some((_, message)) => message,
            None => format!("no readable transcript event in {}", path.display()),
        }));
    }
    Ok(())
}

/// Open a transcript as a byte stream, decompressing zstd-compressed rollouts.
///
/// Codex rotates older sessions into `.jsonl.zst`; reading them as plain bytes
/// would report a binary file where the dialogue actually lives.
fn open_transcript(path: &Path) -> Result<Box<dyn std::io::Read>, ReadError> {
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    if path.extension().is_some_and(|ext| ext == "zst") {
        let decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|e| ReadError(format!("read {}: {e}", path.display())))?;
        return Ok(Box::new(decoder));
    }
    Ok(Box::new(file))
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
    let reader = open_transcript(&path)?;
    let mut parsed = 0usize;
    let mut first_error: Option<(usize, String)> = None;
    for (index, line) in BufReader::new(reader).lines().enumerate() {
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
        let event: Value = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(error) => {
                let message = format!(
                    "invalid transcript JSON at {}:{}: {error}",
                    path.display(),
                    index + 1
                );
                if first_error.is_none() {
                    first_error = Some((index + 1, message));
                }
                page.warnings.push(format!(
                    "skipped unreadable transcript line {}:{}: {error}",
                    path.display(),
                    index + 1
                ));
                continue;
            }
        };
        parsed += 1;
        if accept(&mut page, event, index, limit, selected)? {
            break;
        }
    }
    if parsed == 0 && page.records.is_empty() {
        if let Some((_, message)) = first_error {
            return Err(ReadError(message));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    fn located(source: Source, path: PathBuf) -> Located {
        Located { source, path }
    }

    fn collect(source: Source, path: &Path) -> (Vec<Value>, Vec<String>) {
        let mut rows = Vec::new();
        let mut warnings = Vec::new();
        stream_dialogue(
            &located(source, path.to_path_buf()),
            "id",
            &mut warnings,
            |line, event| {
                rows.push(json!({"line": line, "event": event}));
                Ok(())
            },
        )
        .expect("the transcript is readable");
        (rows, warnings)
    }

    /// A transcript that lost one line to a partial write must still yield the
    /// dialogue around it: the lost line is reported, the session is not lost.
    #[test]
    fn one_unreadable_line_does_not_cost_the_whole_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for index in 0..5 {
            let kind = if index % 2 == 0 {
                "USER_INPUT"
            } else {
                "PLANNER_RESPONSE"
            };
            writeln!(
                file,
                "{{\"type\":\"{kind}\",\"created_at\":\"2020-01-01T00:00:0{index}Z\",\"content\":\"line {index}\"}}"
            )
            .unwrap();
            if index == 2 {
                // A fragment whose head was lost: not parseable as an event.
                writeln!(file, "ind task\",\"toolSummary\":\"Kill task\"}}]}}").unwrap();
            }
        }
        drop(file);

        let (rows, warnings) = collect(Source::Antigravity, &path);
        assert_eq!(rows.len(), 5, "every readable line still yields dialogue");
        assert_eq!(warnings.len(), 1, "the unreadable line is reported");
        assert!(warnings[0].contains("skipped unreadable transcript line"));
    }

    /// A transcript that yields no event at all stays unreadable, and the
    /// failure keeps the line provenance without echoing the corrupt source.
    #[test]
    fn a_transcript_without_any_readable_event_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        fs::write(&path, b"not json at all\nstill not json\n").unwrap();
        let mut warnings = Vec::new();
        let error = stream_dialogue(
            &located(Source::Antigravity, path.clone()),
            "id",
            &mut warnings,
            |_, _| Ok(()),
        )
        .expect_err("a transcript with no event is unreadable");
        let message = error.to_string();
        assert!(
            message.contains("invalid transcript JSON"),
            "the failure names the problem: {message}"
        );
        assert!(
            message.contains("transcript.jsonl:1"),
            "the failure keeps line provenance: {message}"
        );
        assert!(
            !message.contains("not json"),
            "the failure never echoes the corrupt source: {message}"
        );
    }

    /// Codex rotates older rollouts into `.jsonl.zst`; they must stay readable.
    #[test]
    fn a_compressed_rollout_is_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl.zst");
        let file = std::fs::File::create(&path).unwrap();
        let encoder = zstd::stream::write::Encoder::new(file, 3).unwrap();
        let mut encoder = encoder.auto_finish();
        writeln!(
            encoder,
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"compressed question\"}}]}}}}"
        )
        .unwrap();
        writeln!(
            encoder,
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"compressed answer\"}}]}}}}"
        )
        .unwrap();
        drop(encoder);

        let (rows, warnings) = collect(Source::Codex, &path);
        assert_eq!(rows.len(), 2, "both compressed events are read");
        assert!(
            warnings.is_empty(),
            "a healthy compressed rollout warns nothing"
        );
    }

    /// A rotated Antigravity conversation database holds no dialogue at all:
    /// that is an empty extraction, not a read failure.
    #[test]
    fn a_rotated_antigravity_database_yields_no_dialogue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conversation.db");
        let output = std::process::Command::new("sqlite3")
            .arg(&path)
            .arg("CREATE TABLE steps (idx INTEGER); CREATE TABLE gen_metadata (idx INTEGER);")
            .output()
            .expect("sqlite3");
        assert!(output.status.success());

        let (rows, warnings) = collect(Source::Antigravity, &path);
        assert!(rows.is_empty(), "a rotated database holds no dialogue");
        assert_eq!(warnings.len(), 1, "the empty extraction is reported");
        assert!(warnings[0].contains("holds no dialogue"));
    }
}

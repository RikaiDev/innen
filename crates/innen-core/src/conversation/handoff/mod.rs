//! Handoff digest for sources whose transcript cannot be briefed from message
//! roles alone (Antigravity). It compresses by information content, not by
//! position: the user's own requests carry the decisions and corrections, so
//! every one is kept; the assistant's prose is mostly derivable from them and is
//! replaced by pointers the prose cannot be trusted to supply -- what exists on
//! disk, what was searched, what failed, and what the assistant merely *claims*.
//! Nothing here is summarised or inferred: each entry is a source fact with its
//! line, so a reader can expand it with `--lines`.

use std::collections::BTreeMap;
use std::io::BufReader;

use serde::Serialize;
use serde_json::Value;

use super::bounded::{line_ranges, read_capped_line, LINE_BUDGET};
use super::projection;

mod claims;
mod paths;

use super::read::io_error;
use super::record::{Brief, EvidencePointer, Record};
use super::{Located, ReadError, Source};
use claims::collect_claims;
use paths::{group_files, paths_in, A, C, W};

const MAX_TURNS: usize = 100;
const MAX_TURN_CHARS: usize = 800;
const MAX_FILES: usize = 60;
const MAX_SEARCHES: usize = 40;
const MAX_CLAIMS: usize = 24;
const MAX_ERRORS: usize = 50;

#[derive(Debug, Default, Serialize)]
pub struct Handoff {
    /// `[source line, text]` for each user request, envelope removed.
    pub user_turns: Vec<(usize, String)>,
    /// Directory shared by every absolute artifact path; each `artifacts`
    /// directory is relative to it (`"."` is the root itself). Empty when the
    /// paths share nothing, and then directories are shown in full.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub artifact_root: String,
    /// `[directory, ["name[flags]"]]`; flags: W written by a tool, C named in a
    /// command, A named in assistant text. A path only the assistant mentions is
    /// a claim, not evidence that the file exists.
    pub artifacts: Vec<(String, Vec<String>)>,
    /// `[source line, query]` for each web search.
    pub searches: Vec<(usize, String)>,
    pub error_lines: Vec<usize>,
    /// `[source line, snippet]`: status or authority the assistant asserted.
    /// Unverified by construction; the transcript holds no proof of them.
    pub assistant_claims: Vec<(usize, String)>,
}

#[derive(Default)]
struct Scan {
    files: BTreeMap<String, u8>,
    truncated: Vec<usize>,
    claims_seen: usize,
    searches_seen: usize,
    errors_seen: usize,
}

/// Fill `out.latest_user` and `out.handoff`; return the unanswered-request
/// pointer when the newest user request has no assistant text after it.
pub(super) fn antigravity(
    located: &Located,
    out: &mut Brief,
) -> Result<Option<EvidencePointer>, ReadError> {
    let file = std::fs::File::open(&located.path).map_err(|e| io_error(&located.path, e))?;
    let mut reader = BufReader::new(file);
    let mut index = 0usize;
    let mut handoff = Handoff::default();
    let mut scan = Scan::default();
    let mut turns_seen = 0usize;
    let (mut tool_steps, mut tool_results, mut errors, mut last_line) = (0usize, 0usize, 0usize, 0);
    let mut last_text_line = 0usize;
    while let Some(line) =
        read_capped_line(&mut reader, LINE_BUDGET).map_err(|e| io_error(&located.path, e))?
    {
        index += 1;
        if line.oversized {
            out.omitted_records += 1;
            out.coverage_incomplete = true;
            out.warnings.push(format!(
                "omitted oversized source line {index}; expand it with --lines"
            ));
            continue;
        }
        let Ok(event) = serde_json::from_slice::<Value>(&line.bytes) else {
            if !line.bytes.iter().all(u8::is_ascii_whitespace) {
                out.coverage_incomplete = true;
                out.warnings
                    .push(format!("skipped unreadable source line {index}"));
            }
            continue;
        };
        let kind = event["type"].as_str().unwrap_or("").to_owned();
        let text = event["content"].as_str().unwrap_or("").to_owned();
        let has_text = !text.trim().is_empty();
        match kind.as_str() {
            "USER_INPUT" => {
                if let Some(mut record) = shown(event, index) {
                    let body = request_text(record.event["content"].as_str().unwrap_or(""));
                    record.event["content"] = Value::String(body.clone());
                    if has_truncation(&record.event) {
                        scan.truncated.push(index);
                    }
                    turns_seen += 1;
                    handoff.user_turns.push((index, cap_turn(&body, index)));
                    out.latest_user = Some(record);
                }
                (tool_steps, tool_results, errors, last_line) = (0, 0, 0, index);
                continue;
            }
            "PLANNER_RESPONSE" => {
                read_tool_calls(&event, index, &mut scan, &mut handoff);
                if has_text {
                    last_text_line = index;
                    if has_truncation(&event) {
                        scan.truncated.push(index);
                    }
                    for path in paths_in(&text) {
                        *scan.files.entry(path).or_default() |= A;
                    }
                    collect_claims(&text, index, &mut scan, &mut handoff);
                } else {
                    tool_steps += 1;
                }
            }
            "GENERIC" => tool_results += 1,
            "ERROR_MESSAGE" => {
                errors += 1;
                scan.errors_seen += 1;
                if handoff.error_lines.len() < MAX_ERRORS {
                    handoff.error_lines.push(index);
                }
            }
            _ => continue,
        }
        last_line = index;
    }

    if handoff.user_turns.len() > MAX_TURNS {
        let drop = handoff.user_turns.len() - MAX_TURNS;
        handoff.user_turns.drain(..drop);
        out.omitted_records += drop;
        out.warnings.push(format!(
            "showing the last {MAX_TURNS} of {turns_seen} user requests; older ones: --lines"
        ));
    }
    (handoff.artifact_root, handoff.artifacts) = group_files(scan.files, &mut out.warnings);
    overflow(
        &mut out.warnings,
        "searches",
        scan.searches_seen,
        MAX_SEARCHES,
    );
    overflow(
        &mut out.warnings,
        "assistant claims",
        scan.claims_seen,
        MAX_CLAIMS,
    );
    overflow(
        &mut out.warnings,
        "error lines",
        scan.errors_seen,
        MAX_ERRORS,
    );
    if !scan.truncated.is_empty() {
        out.warnings.push(format!(
            "source text truncated in {} shown events (lines {}); this reader cannot restore it",
            scan.truncated.len(),
            line_ranges(&scan.truncated)
        ));
    }
    out.handoff = Some(handoff);

    let Some(user) = out.latest_user.as_ref() else {
        return Ok(None);
    };
    let answered = last_text_line > user.line;
    if tool_steps + tool_results + errors > 0 {
        out.warnings.push(format!(
            "after line {}: {tool_steps} tool-only planner steps, {tool_results} tool results, {errors} errors (lines {}-{last_line}); read with --view events --offset {}",
            user.line,
            user.line,
            user.line - 1
        ));
    }
    Ok((!answered).then(|| EvidencePointer {
        line: user.line,
        marker: "user_request_unanswered".into(),
        basis: "no_text_planner_response_after_latest_user_request".into(),
        // The request itself is the last of `handoff.user_turns`; a copy here
        // would state the same fact twice.
        preview: String::new(),
    }))
}

fn overflow(warnings: &mut Vec<String>, what: &str, seen: usize, cap: usize) {
    if seen > cap {
        warnings.push(format!(
            "{what}: showing {cap} of {seen}; expand with --view events"
        ));
    }
}

fn shown(event: Value, line: usize) -> Option<Record> {
    let event = projection::dialogue(Source::Antigravity, event)
        .ok()
        .flatten()?;
    Some(Record { line, event })
}

/// The user's own words: the text inside `<USER_REQUEST>`, wherever the
/// surrounding envelopes (workspace, metadata, settings) sit.
fn request_text(content: &str) -> String {
    let inner = content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, tail)| tail.split_once("</USER_REQUEST>"))
        .map_or(content, |(inner, _)| inner);
    inner.trim().to_owned()
}

fn has_truncation(event: &Value) -> bool {
    event["truncated_fields"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
}

fn cap_turn(text: &str, line: usize) -> String {
    let text = text.trim();
    let total = text.chars().count();
    if total <= MAX_TURN_CHARS {
        return text.to_owned();
    }
    let head: String = text.chars().take(MAX_TURN_CHARS).collect();
    format!(
        "{head}...(+{} chars; --lines {line})",
        total - MAX_TURN_CHARS
    )
}

/// A tool-call argument arrives as a JSON-encoded string: `"\"/a/b.pdf\""`.
fn arg(call: &Value, key: &str) -> Option<String> {
    let raw = call["args"][key].as_str()?;
    Some(serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_owned()))
}

fn read_tool_calls(event: &Value, line: usize, scan: &mut Scan, handoff: &mut Handoff) {
    for call in event["tool_calls"].as_array().into_iter().flatten() {
        match call["name"].as_str().unwrap_or("") {
            "write_file" | "edit_file" => {
                if let Some(path) = arg(call, "Path") {
                    *scan.files.entry(path).or_default() |= W;
                }
            }
            "run_command" => {
                for path in paths_in(&arg(call, "CommandLine").unwrap_or_default()) {
                    *scan.files.entry(path).or_default() |= C;
                }
            }
            "search_web" => {
                if let Some(query) = arg(call, "query") {
                    scan.searches_seen += 1;
                    if handoff.searches.len() < MAX_SEARCHES {
                        handoff.searches.push((line, query));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_text_drops_envelopes_wherever_they_sit() {
        let wrapped = "<user_information>ws</user_information>\n<USER_REQUEST>\n改版\n</USER_REQUEST>\n<ADDITIONAL_METADATA>t</ADDITIONAL_METADATA>";
        assert_eq!(request_text(wrapped), "改版");
        assert_eq!(request_text("no envelope"), "no envelope");
    }
}

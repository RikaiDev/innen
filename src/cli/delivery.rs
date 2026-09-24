//! Read-only delivery candidates from a native coding-tool session.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const MAX_SESSION_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SESSION_RECORDS: usize = 20_000;

fn linked_paths(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for part in text.split("](<").skip(1) {
        let Some((raw, _)) = part.split_once(">)") else {
            continue;
        };
        let path = PathBuf::from(raw);
        if path.is_absolute() && super::local_discovery::is_document(&path) {
            out.push(path);
        }
    }
    out
}

pub(super) fn inspect(
    source_root: Option<&Path>,
    source: &str,
    session: &str,
) -> Result<Value, String> {
    if source != "codex" {
        return Err("delivery candidate extraction currently supports --source codex".into());
    }
    let limits = innen_core::conversation::scan::ScanLimits {
        max_bytes: MAX_SESSION_BYTES,
        max_records: MAX_SESSION_RECORDS,
    };
    let scan = innen_core::conversation::scan::scan(source_root, source, session, &limits)
        .map_err(|e| e.to_string())?;
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for record in &scan.records {
        let event = &record.event;
        if event["role"] != "assistant" {
            continue;
        }
        if event["phase"] != "final_answer" && event["phase"] != "final" {
            continue;
        }
        let Some(text) = event["content"].as_str() else {
            continue;
        };
        for path in linked_paths(text) {
            if !seen.insert(path.clone()) {
                continue;
            }
            let exists = path.is_file();
            let relocated = if exists {
                None
            } else {
                let clue = path.file_name().unwrap_or_default().to_string_lossy();
                Some(super::local_discovery::discover(
                    &clue,
                    &super::local_discovery::default_roots(),
                ))
            };
            let mut relocated_candidates = relocated
                .as_ref()
                .map(|r| r.candidates.clone())
                .unwrap_or_default();
            if let Some(expected_name) = path.file_name().and_then(|name| name.to_str()) {
                let exact: Vec<Value> = relocated_candidates
                    .iter()
                    .filter(|candidate| {
                        Path::new(candidate["path"].as_str().unwrap_or(""))
                            .file_name()
                            .and_then(|name| name.to_str())
                            == Some(expected_name)
                    })
                    .cloned()
                    .collect();
                if !exact.is_empty() {
                    relocated_candidates = exact;
                }
            }
            candidates.push(json!({
                    "mentioned_path": path,
                    "exists": exists,
                    "relocated_candidates": relocated_candidates,
                    "relocation_search_complete": relocated.as_ref().map(|r| r.complete),
                    "relocation_results_truncated": relocated.as_ref().map(|r| r.results_truncated),
                "source_line": record.line,
                "status": "requires_artifact_review"
            }));
        }
    }
    Ok(json!({
        "source": source,
        "session": session,
        "source_path": scan.source_path,
        "complete": scan.complete && !scan.projection_incomplete,
        "warnings": scan.warnings,
        "delivery_candidates": candidates
    }))
}

#[cfg(test)]
mod tests {
    use super::linked_paths;

    #[test]
    fn extracts_only_absolute_document_links() {
        let found = linked_paths("[done](</Users/test/甲研究院報價單.docx>) [web](https://example.com) [relative](<out.pdf>)");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].to_string_lossy(),
            "/Users/test/甲研究院報價單.docx"
        );
    }
}

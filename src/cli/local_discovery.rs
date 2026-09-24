//! Bounded, read-only filename discovery for graph misses.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 12;
const MAX_RESULTS: usize = 8;

pub(super) struct DiscoveryResult {
    pub candidates: Vec<Value>,
    pub complete: bool,
    pub scanned_entries: usize,
    pub matching_files: usize,
    pub results_truncated: bool,
}

fn terms(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut cjk = String::new();
    let mut ascii = String::new();
    let flush_cjk = |run: &mut String, out: &mut BTreeSet<String>| {
        let chars: Vec<char> = run.chars().collect();
        for pair in chars.windows(2) {
            out.insert(pair.iter().collect());
        }
        run.clear();
    };
    let flush_ascii = |run: &mut String, out: &mut BTreeSet<String>| {
        if run.len() >= 3 {
            out.insert(run.to_lowercase());
        }
        run.clear();
    };
    for ch in text.chars() {
        if ('\u{3400}'..='\u{9fff}').contains(&ch) {
            flush_ascii(&mut ascii, &mut out);
            cjk.push(ch);
        } else if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            ascii.push(ch);
        } else {
            flush_cjk(&mut cjk, &mut out);
            flush_ascii(&mut ascii, &mut out);
        }
    }
    flush_cjk(&mut cjk, &mut out);
    flush_ascii(&mut ascii, &mut out);
    out
}

pub(super) fn is_document(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("pdf" | "docx" | "doc" | "xlsx" | "xls" | "odt" | "ods" | "pptx" | "ppt")
    )
}

pub(super) fn discover(query: &str, roots: &[PathBuf]) -> DiscoveryResult {
    let query_terms = terms(query);
    if query_terms.len() < 2 {
        return DiscoveryResult {
            candidates: Vec::new(),
            complete: true,
            scanned_entries: 0,
            matching_files: 0,
            results_truncated: false,
        };
    }
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().cloned().map(|p| (p, 0)).collect();
    let mut seen = 0;
    let mut complete = true;
    let mut hits: Vec<(usize, PathBuf)> = Vec::new();
    while let Some((dir, depth)) = queue.pop_front() {
        if seen >= MAX_ENTRIES || depth > MAX_DEPTH {
            complete = false;
            break;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            complete = false;
            continue;
        };
        let mut entries = Vec::new();
        let remaining = MAX_ENTRIES.saturating_sub(seen);
        for entry in read_dir.take(remaining + 1) {
            match entry {
                Ok(entry) => entries.push(entry),
                Err(_) => complete = false,
            }
        }
        if entries.len() > remaining {
            complete = false;
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            seen += 1;
            if seen > MAX_ENTRIES {
                complete = false;
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.starts_with("~$") {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH
                    && !matches!(
                        name.as_str(),
                        "node_modules" | "vendor" | "venv" | "site-packages" | "target"
                    )
                {
                    queue.push_back((path, depth + 1));
                } else if depth >= MAX_DEPTH {
                    complete = false;
                }
            } else if kind.is_file() && is_document(&path) {
                let file_terms = terms(&name);
                let shared: Vec<&String> = query_terms.intersection(&file_terms).collect();
                let overlap = shared.len();
                let has_specific_clue = shared.iter().any(|term| {
                    !matches!(
                        term.as_str(),
                        "報價" | "價單" | "估價" | "合約" | "文件" | "資料"
                    )
                });
                if overlap >= 2 && has_specific_clue {
                    hits.push((overlap, path));
                }
            }
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let matching_files = hits.len();
    let candidates = hits.into_iter()
        .take(MAX_RESULTS)
        .map(|(matched_terms, path)| json!({"path": path, "matched_terms": matched_terms, "status": "unindexed_local_file"}))
        .collect();
    DiscoveryResult {
        candidates,
        complete,
        scanned_entries: seen.min(MAX_ENTRIES),
        matching_files,
        results_truncated: matching_files > MAX_RESULTS,
    }
}

pub(super) fn default_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    ["Documents", "Downloads"]
        .iter()
        .map(|name| PathBuf::from(&home).join(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::discover;

    #[test]
    fn finds_unindexed_docx_from_natural_request() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("醫療健康/甲研究院");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("甲大學_甲研究院_智慧系統開發報價單_20260813.docx");
        std::fs::write(&file, b"fixture").unwrap();
        std::fs::write(
            dir.join("~$甲大學_甲研究院_智慧系統開發報價單_20260813.docx"),
            b"lock",
        )
        .unwrap();
        std::fs::write(dir.join("花蓮報價單.docx"), b"fixture").unwrap();
        std::fs::write(dir.join("甲大學_丙企業_報價單.docx"), b"fixture").unwrap();
        let result = discover("之前開給甲研究院的報價單", &[temp.path().to_path_buf()]);
        let hits = result.candidates;
        assert!(result.complete);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["path"], file.to_string_lossy().as_ref());
        let hits = discover("甲研究院 甲大學 報價", &[temp.path().to_path_buf()]).candidates;
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["path"], file.to_string_lossy().as_ref());
        let missing = discover("甲研究院 報價", &[temp.path().join("unavailable")]);
        assert!(!missing.complete);
        assert!(missing.candidates.is_empty());
    }
}

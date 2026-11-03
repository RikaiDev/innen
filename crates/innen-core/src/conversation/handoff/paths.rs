//! Deliverable paths named in a transcript: found, decoded, merged, grouped.

use std::collections::BTreeMap;

use super::{overflow, MAX_FILES};

const FILE_EXTENSIONS: [&str; 13] = [
    "pptx", "pdf", "docx", "xlsx", "csv", "png", "jpg", "jpeg", "svg", "html", "md", "py", "json",
];

pub(super) const W: u8 = 1;
pub(super) const C: u8 = 2;
pub(super) const A: u8 = 4;

fn is_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '\\'
                | ','
                | ';'
                | '('
                | ')'
                | '['
                | ']'
                | '<'
                | '>'
                | '*'
                | '|'
                | '='
                | '，'
                | '；'
                | '：'
                | '（'
                | '）'
                | '「'
                | '」'
                | '『'
                | '』'
                | '、'
                | '。'
        )
}

fn percent_decode(token: &str) -> String {
    let bytes = token.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| token.to_owned())
}

/// Path-like tokens naming a deliverable file: contain `/`, end in a known
/// document or image extension. Markdown links are percent-decoded.
pub(super) fn paths_in(text: &str) -> Vec<String> {
    text.split(is_delimiter)
        .map(|t| t.trim_end_matches(['.', ':']))
        .map(|t| t.trim_start_matches("file://").trim_start_matches("file:"))
        .filter(|t| t.contains('/'))
        .map(percent_decode)
        .filter(|t| {
            t.rsplit_once('.')
                .is_some_and(|(_, ext)| FILE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
        })
        .collect()
}

/// `//a/b` and `/a//b` are the same path.
fn tidy(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        out.push('/');
        out.push_str(part);
    }
    if path.starts_with('/') {
        out
    } else {
        out.trim_start_matches('/').to_owned()
    }
}

fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Longest directory shared by all of `dirs` (component-wise), at least two
/// components deep so that `/` or `/Users` alone is never chosen.
fn common_root(dirs: &[&str]) -> String {
    let mut parts: Vec<&str> = dirs.first().map_or(Vec::new(), |d| d.split('/').collect());
    for dir in dirs {
        let next: Vec<&str> = dir.split('/').collect();
        let keep = parts.iter().zip(&next).take_while(|(a, b)| a == b).count();
        parts.truncate(keep);
    }
    if parts.len() < 3 {
        return String::new();
    }
    parts.join("/")
}

/// Group files by directory. A relative path whose directory is the tail of
/// exactly one known absolute directory is the same file and moves there; a
/// root shared by every absolute directory is stated once.
pub(super) fn group_files(
    files: BTreeMap<String, u8>,
    warnings: &mut Vec<String>,
) -> (String, Vec<(String, Vec<String>)>) {
    let files: BTreeMap<String, u8> = files.into_iter().fold(BTreeMap::new(), |mut acc, (p, f)| {
        *acc.entry(tidy(&p)).or_default() |= f;
        acc
    });
    let absolute_dirs: Vec<String> = files
        .keys()
        .filter(|p| p.starts_with('/'))
        .map(|p| split_path(p).0.to_owned())
        .collect();
    let mut merged: BTreeMap<String, u8> = BTreeMap::new();
    for (path, flags) in files {
        let (dir, name) = split_path(&path);
        let mut key = path.clone();
        if !path.starts_with('/') && !dir.is_empty() {
            let tail = format!("/{dir}");
            let mut hits = absolute_dirs.iter().filter(|d| d.ends_with(&tail));
            let first = hits.next();
            if let (Some(abs), true) = (first, hits.all(|d| Some(d) == first)) {
                key = format!("{abs}/{name}");
            }
        }
        *merged.entry(key).or_default() |= flags;
    }
    let total = merged.len();
    let kept: Vec<(String, u8)> = merged.into_iter().take(MAX_FILES).collect();
    let abs_dirs: Vec<&str> = kept
        .iter()
        .filter(|(p, _)| p.starts_with('/'))
        .map(|(p, _)| split_path(p).0)
        .collect();
    let root = common_root(&abs_dirs);
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, flags) in &kept {
        let (dir, name) = split_path(path);
        let shown = match dir.strip_prefix(root.as_str()) {
            Some(rest) if !root.is_empty() => rest.trim_start_matches('/'),
            _ => dir,
        };
        let shown = if shown.is_empty() && !root.is_empty() && path.starts_with('/') {
            "."
        } else {
            shown
        };
        let tag: String = [(W, 'W'), (C, 'C'), (A, 'A')]
            .iter()
            .filter(|(bit, _)| flags & bit != 0)
            .map(|(_, c)| *c)
            .collect();
        grouped
            .entry(shown.to_owned())
            .or_default()
            .push(format!("{name}[{tag}]"));
    }
    overflow(warnings, "files", total, MAX_FILES);
    (root, grouped.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_decoded_and_filtered_to_deliverables() {
        let text = "見 [簡報](innen-wiki/out/%E7%B0%A1%E5%A0%B1.pdf) 與 `/a/b/deck.pptx`，另有 notes.txt 與 plain.pdf";
        assert_eq!(
            paths_in(text),
            vec!["innen-wiki/out/簡報.pdf", "/a/b/deck.pptx"]
        );
    }

    #[test]
    fn a_relative_path_merges_into_the_absolute_directory_it_ends() {
        let mut files = BTreeMap::new();
        files.insert("/r/innen-wiki/out/deck.pdf".to_owned(), A);
        files.insert("innen-wiki/out/deck.pdf".to_owned(), C);
        files.insert("innen-wiki/out/more.pdf".to_owned(), A);
        files.insert("//r/innen-wiki/other/x.md".to_owned(), W);
        let mut warnings = Vec::new();
        let (root, grouped) = group_files(files, &mut warnings);
        assert_eq!(root, "/r/innen-wiki");
        assert_eq!(
            grouped,
            vec![
                ("other".to_owned(), vec!["x.md[W]".to_owned()]),
                (
                    "out".to_owned(),
                    vec!["deck.pdf[CA]".to_owned(), "more.pdf[A]".to_owned()]
                ),
            ]
        );
    }

    #[test]
    fn a_shared_root_needs_two_real_components() {
        assert_eq!(common_root(&["/Users/a", "/Users/b"]), "");
        assert_eq!(common_root(&["/Users/a/x", "/Users/a/y"]), "/Users/a");
    }
}

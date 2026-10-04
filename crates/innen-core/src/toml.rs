//! Flat `key = "value"` TOML reader.
//!
//! `config.rs` and `parity/profile.rs` both needed this and each carried its
//! own copy of `strip_comment` plus `unquote`. The behaviour is identical:
//! `#` outside double quotes starts a comment, values are double-quoted with
//! `\\` and `\"` escapes only, a bare interior quote is rejected, a leading
//! BOM is stripped, and a non-string or unparsable value yields `None`.
//!
//! Section headers are ignored here. `config.rs` tracks them because `[core]`
//! and `[index]` gate which keys it accepts; `profile.toml` is flat, so the
//! caller wants every key it recognises.

/// Strip a trailing `#` comment that is not inside double quotes.
pub(crate) fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Skip the escaped char so `\"` does not toggle quoting.
            b'\\' if in_quotes => i += 2,
            b'"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            b'#' if !in_quotes => return line[..i].trim_end(),
            _ => i += 1,
        }
    }
    line
}

/// Unwrap a double-quoted value, resolving `\\` and `\"`.
///
/// Returns `None` for an unquoted value, an unterminated quote, a bare
/// interior quote, or any other escape.
pub(crate) fn unquote(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            }
        } else if c == '"' {
            // Bare interior quote: neither an escape nor the closing pair.
            return None;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Every `key = "value"` pair, last duplicate winning.
///
/// Section headers, non-string values, and unparsable values are skipped
/// without error. A leading BOM is stripped so the first key still parses.
pub(crate) fn string_entries(text: &str) -> Vec<(String, String)> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut out: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let Some(value) = unquote(line[eq + 1..].trim()) else {
            continue;
        };
        match out.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => out.push((key.to_string(), value)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_outside_quotes_only() {
        assert_eq!(strip_comment("a = \"b\" # tail"), "a = \"b\"");
        assert_eq!(strip_comment("a = \"b#c\""), "a = \"b#c\"");
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(strip_comment("a = \"b\\\""), "a = \"b\\\"");
    }

    #[test]
    fn unquote_resolves_only_two_escapes() {
        assert_eq!(unquote("\"plain\"").as_deref(), Some("plain"));
        assert_eq!(unquote(r#""a\"b""#).as_deref(), Some(r#"a"b"#));
        assert_eq!(unquote(r#""a\\b""#).as_deref(), Some(r"a\b"));
        // Unquoted, unterminated, bare interior quote, unknown escape.
        assert_eq!(unquote("plain"), None);
        assert_eq!(unquote("\"open"), None);
        assert_eq!(unquote("\"a\"b\""), None);
        assert_eq!(unquote(r#""a\nb""#), None);
        assert_eq!(unquote("\"\"").as_deref(), Some(""));
    }

    #[test]
    fn entries_skip_sections_and_keep_last_duplicate() {
        let got = string_entries(
            "\u{FEFF}[core]\ntitle = \"one\"\nn = 5\nbad = \"a\"b\"\ntitle = \"two\"\n",
        );
        assert_eq!(got, vec![("title".to_string(), "two".to_string())]);
    }

    #[test]
    fn entries_ignore_unknown_keys_only_at_the_call_site() {
        let got = string_entries("title = \"t\"\nblurb = \"b\"\nstatus = \"s\"\nextra = \"x\"\n");
        assert_eq!(got.len(), 4, "reader returns every key: {got:?}");
    }
}

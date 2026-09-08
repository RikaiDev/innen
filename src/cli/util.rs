pub(super) fn is_human(format: &str) -> bool {
    format == "human"
}

// TSV field budget (chars, not bytes — CJK safe) for human tables.
const TSV_FIELD_LIMIT: usize = 200;

/// Escape one human-table TSV field: truncate to [`TSV_FIELD_LIMIT`] chars
/// (CJK-safe, chars not bytes) with a `…` marker, then encode `\t`→`\\t`,
/// `\n`→`\\n`, `\r`→`\\r` so embedded tabs/newlines cannot break rows.
/// Truncation runs first so escape sequences stay intact.
pub(super) fn escape_tsv_field(s: &str) -> String {
    let truncated: String = if s.chars().count() > TSV_FIELD_LIMIT {
        let mut out: String = s.chars().take(TSV_FIELD_LIMIT).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    };
    truncated
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::{escape_tsv_field, TSV_FIELD_LIMIT};
    #[test]
    fn tsv_escape_pins_tabs_newlines_and_cjk_truncation() {
        assert_eq!(escape_tsv_field("a\tb\nc\rd"), "a\\tb\\nc\\rd");
        let long = "字".repeat(250);
        let got = escape_tsv_field(&long);
        assert_eq!(got, format!("{}…", "字".repeat(TSV_FIELD_LIMIT)));
        assert_eq!(got.chars().count(), TSV_FIELD_LIMIT + 1);
        assert_eq!(escape_tsv_field("臺北"), "臺北");
    }
}

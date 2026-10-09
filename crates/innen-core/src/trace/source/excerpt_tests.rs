//! The excerpt window must be found in the text it will be cut from.
//!
//! Lowercasing is not byte-length preserving: `İ` (U+0130) is two bytes and
//! lowercases to `i` plus a combining dot (three bytes). Searching the
//! lowercased string and slicing the original therefore slices at an offset
//! belonging to a different string.

use super::excerpt;

#[test]
fn excerpt_window_starts_on_the_match_when_lowercasing_changes_byte_length() {
    // `İ` is two bytes and lowercases to three, so each one drifts the offset
    // found in the lowercased text by a byte. 130 of them drift it past the
    // 120-byte lookback, so the window begins *after* the match and the excerpt
    // comes back without the term the caller searched for.
    //
    // Trailing content is required: without it the `at.min(text.len())` clamp
    // pulls the drifted offset back inside the document and hides the defect.
    let mut text = "İ".repeat(130);
    text.push_str("NEEDLE");
    text.push_str(&"z".repeat(600));
    let out = excerpt(&text, &["needle".to_string()]);
    assert!(
        out.contains("NEEDLE"),
        "the excerpt window must contain the matched text: {} bytes, head {:?}",
        text.len(),
        &out[..out.len().min(40)]
    );
}

#[test]
fn excerpt_window_starts_on_the_match_for_a_short_non_ascii_document() {
    // The same drift with a single dotted capital. The lookback absorbs it here,
    // so this case passes both before and after the fix; it is kept because it
    // is the smallest input that shows the offset belongs to another string.
    let text = "İstanbul is a city";
    let out = excerpt(text, &["stanbul".to_string()]);
    assert!(
        out.contains("stanbul"),
        "the excerpt window must contain the matched text: {out:?}"
    );
}

#[test]
fn excerpt_window_starts_on_the_match_for_ascii() {
    let mut text = "a".repeat(200);
    text.push_str("NEEDLE");
    text.push_str(&"b".repeat(600));
    let out = excerpt(&text, &["needle".to_string()]);
    assert!(
        out.contains("NEEDLE"),
        "case-insensitive match on plain ascii must land on the needle: {out:?}"
    );
}

#[test]
fn excerpt_still_reports_a_short_document_in_full() {
    let out = excerpt("short text", &["nothing-here".to_string()]);
    assert_eq!(out, "short text");
}

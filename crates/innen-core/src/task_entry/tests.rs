use serde_json::json;

use super::admission::admit_literal_candidate;

#[test]
fn multi_term_request_rejects_unrelated_quote() {
    let unrelated = json!({"label": "丙企業_花蓮報價單.pdf"});
    let matching = json!({"label": "甲研究院_甲大學_報價單.pdf"});
    let terms = ["甲研究院", "甲大學", "報價"];
    assert!(!admit_literal_candidate(&unrelated, &terms, false));
    assert!(admit_literal_candidate(&matching, &terms, false));
}

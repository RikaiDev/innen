//! Roundtrip, boundary and attack tests for the conversation-page grammar.

use super::compress::chunks;
use super::{decode, encode, tokens, ENCODING};
use serde_json::json;

#[test]
fn exact_copies_across_events_still_count_as_redundancy() {
    let content = (0..50)
        .map(|i| format!("unique source field {i}; "))
        .collect::<String>();
    let original = json!({"records":[{"line":1,"event":{"text":content}},{"line":2,"event":{"text":content}}]});
    let packet = encode(&original, &original).unwrap();
    assert_eq!(decode(&packet).unwrap(), original);
    assert!(tokens(&packet).unwrap() < tokens(&original).unwrap());
}
#[test]
fn conversation_interior_versions_preserve_roles_and_negation() {
    let shared = (0..25)
        .map(|i| format!("file-{i}: {}\n", "unchanged source evidence ".repeat(5)))
        .collect::<String>();
    let records:Vec<_>=(0..8).map(|i|json!({"line":i,"event":{"role":if i%2==0 {"user"} else {"tool"},
        "content":format!("version {i}\n{shared}{}\n{shared}tail {i}",if i==3 {"DO NOT COMMIT"} else {"unknown"})}})).collect();
    let original =
        json!({"records":records,"literal":{"$seq":[0]},"warnings":["coverage incomplete"]});
    let packed = encode(&original, &original).unwrap();
    assert_eq!(packed["encoding"], ENCODING);
    assert_eq!(decode(&packed).unwrap(), original);
    assert!(tokens(&packed).unwrap() < tokens(&original).unwrap());
    let mut bad = packed;
    bad["source_sha256"] = json!("wrong");
    assert!(decode(&bad).is_err());
}
#[test]
fn short_unicode_and_tag_collisions_do_not_grow() {
    let original = json!({"text":"中文👩‍💻\r\n不得改成成功","x":{"$seq":[0]},"status":null});
    let packet = encode(&original, &original).unwrap();
    assert_eq!(decode(&packet).unwrap(), original);
    assert!(tokens(&packet).unwrap() <= tokens(&original).unwrap());
}
#[test]
fn cyclic_forward_and_expansion_attacks_fail() {
    let p = json!({"encoding":ENCODING,"join_key":"$seq","dictionary":[[0]],"page":{},"source_sha256":"x"});
    assert!(decode(&p).is_err());
    let p = json!({"encoding":ENCODING,"join_key":"$seq","dictionary":["x"],"page":{"$seq":[3]},"source_sha256":"x"});
    assert!(decode(&p).is_err());
}
#[test]
fn chunking_is_exact_across_utf8_and_single_line_changes() {
    let s = format!("{}CHANGED{}", "甲乙👩‍💻 ".repeat(100), "prefix ".repeat(100));
    assert_eq!(chunks(&s, false).concat(), s);
    assert_eq!(chunks(&s, true).concat(), s);
}

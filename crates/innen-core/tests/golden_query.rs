//! Golden query contract test (Task 7): byte-exact `expected` replay.

use serde_json::Value;

fn run_query_fixture(case: &Value) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal = innen_core::journal::Journal::open(dir.path()).expect("journal open");
    for ev in case["journal"].as_array().expect("journal array") {
        journal
            .append(
                ev["op"].as_str().expect("op str"),
                ev.get("payload").expect("payload"),
            )
            .expect("append");
    }
    // Quarantine fixture: raw corrupt lines appended directly (bypassing
    // validation) so `query`'s `Journal::open` quarantines them and emits
    // `quarantined_skipped`.
    if let Some(corrupt) = case.get("preseed_corrupt").and_then(|v| v.as_array()) {
        use std::io::Write as _;
        let path = dir.path().join(".innen/journal.jsonl");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open journal for corrupt preseed");
        for line in corrupt {
            let s = line.as_str().expect("corrupt line str");
            f.write_all(s.as_bytes()).expect("write corrupt");
            f.write_all(b"\n").expect("newline");
        }
        f.sync_all().expect("sync");
    }
    let params = innen_core::query::QueryParams {
        q: case["query"]["q"].as_str().expect("q str").to_string(),
        as_of: case["query"]
            .get("as_of")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        limit: case["query"]
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as u16,
        include_expired: case["query"]
            .get("include_expired")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let out = innen_core::query::query(dir.path(), &params).expect("query");
    // Direct struct serialization: field order is the wire order, so the
    // byte comparison is sensitive to it (no `Value` normalization).
    serde_json::to_string(&out).expect("output serializes")
}

#[test]
fn golden_query_byte_exact() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("golden-query.json")).expect("fixture parses");
    assert_eq!(cases.len(), 7, "fixture must hold exactly 7 cases");
    for case in &cases {
        let got = run_query_fixture(case);
        // Expected side via the same struct type: deterministic field order
        // on both sides, byte-exact including wire order.
        let want_struct: innen_core::query::QueryOutput =
            serde_json::from_value(case["expected"].clone()).expect("expected parses");
        let want = serde_json::to_string(&want_struct).expect("expected serializes");
        assert_eq!(got, want, "case {}", case["name"]);
    }
}

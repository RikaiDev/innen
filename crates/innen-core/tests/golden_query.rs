//! Golden query contract test (Task 7): byte-exact `expected` replay.

use serde_json::Value;

fn run_query_fixture(case: &Value) -> Value {
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
    // Normalize through `Value` (BTreeMap key order) so struct field order
    // cannot skew the byte-exact comparison against parsed `expected`.
    serde_json::to_value(&out).expect("output serializes")
}

#[test]
fn golden_query_byte_exact() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("golden-query.json")).expect("fixture parses");
    assert_eq!(cases.len(), 5, "fixture must hold exactly 5 cases");
    for case in &cases {
        let got = run_query_fixture(case);
        let want = serde_json::to_string(&case["expected"]).expect("expected serializes");
        assert_eq!(
            serde_json::to_string(&got).expect("got serializes"),
            want,
            "case {}",
            case["name"]
        );
    }
}

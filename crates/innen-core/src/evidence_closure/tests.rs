use super::contract::{timestamp, Contract, Requirement, SourceGrant};
use super::evaluation::evaluate;
use super::snapshot::snapshot;
use crate::ids::sha256_hex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const AT: &str = "2026-01-03T00:00:00Z";
fn entry(op: &str, payload: Value, at: &str) -> Value {
    json!({"id":sha256_hex(payload.to_string().as_bytes()),"op":op,"payload":payload,"observed_utc":at})
}
fn bytes(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}
fn node(id: &str, text: &str, at: &str) -> Value {
    entry(
        "node.upsert",
        json!({"id":id,"type":"Decision","label":id,"body":text,"provenance":"fixture:source"}),
        at,
    )
}
fn edge(from: &str, to: &str, kind: &str) -> Value {
    entry(
        "edge.assert",
        json!({"from":from,"to":to,"type":kind,"provenance":"fixture:dependency"}),
        AT,
    )
}
fn contract(events: &[Value]) -> Contract {
    let s = snapshot(&bytes(events), AT).unwrap();
    Contract {
        version: 1,
        question: "Which evidence supports the decision?".into(),
        scope_receipt: "fixture:caller-scope".into(),
        scope_nodes: s.graph.nodes.keys().cloned().collect(),
        as_of: AT.into(),
        min_observed_utc: "2026-01-01T00:00:00Z".into(),
        sources: s
            .graph
            .nodes
            .iter()
            .map(|(id, n)| {
                (
                    id.clone(),
                    SourceGrant {
                        node_sha256: sha256_hex(n.to_string().as_bytes()),
                    },
                )
            })
            .collect(),
        facts: vec![Requirement {
            id: "decision".into(),
            description: None,
            alternatives: vec![BTreeSet::from(["claim".into()])],
        }],
        budget_tokens: 10_000,
        max_hops: 4,
        search_limit: 1000,
    }
}
fn fixture() -> Vec<Value> {
    vec![
        node("claim", "Observed claim, not inferred success", AT),
        node("rule", "Applicable rule", AT),
        node("source", "PRIVATE_LEAF_SOURCE", AT),
        node("noise", "Unrelated text", AT),
        edge("claim", "rule", "DEPENDS_ON"),
        edge("rule", "source", "DEPENDS_ON"),
        edge("claim", "noise", "RELATED"),
    ]
}
fn run(e: &[Value], c: &Contract) -> Value {
    evaluate(&bytes(e), c, &c.question).unwrap()
}

#[test]
fn closes_two_hops_shared_evidence_once_and_ignores_related() {
    let e = fixture();
    let mut c = contract(&e);
    c.facts.push(Requirement {
        id: "rule".into(),
        description: None,
        alternatives: vec![BTreeSet::from(["rule".into()])],
    });
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "covered");
    let ids: Vec<_> = out["packet"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["claim", "rule", "source"]);
    assert_eq!(
        out["packet"]["dependency_paths"].as_array().unwrap().len(),
        2
    );
    assert!(
        out["packet"]["evidence"][0]["journal_citations"][0]["line"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[test]
fn denied_second_hop_never_discloses_payload_or_trims_dependency() {
    let e = fixture();
    let mut c = contract(&e);
    c.sources.remove("source");
    c.scope_nodes.remove("source");
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "blocked");
    assert!(out["packet"].is_null());
    assert!(!out.to_string().contains("PRIVATE_LEAF_SOURCE"));
    assert_eq!(
        out["rejected_alternatives"][0]["reason"],
        "scope_or_source_unavailable"
    );
}
#[test]
fn permitted_but_unpinned_dependency_requests_precise_expansion() {
    let e = fixture();
    let mut c = contract(&e);
    c.sources.remove("source");
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "blocked");
    assert_eq!(out["source_requests"][0]["node"], "source");
    assert!(!out.to_string().contains("PRIVATE_LEAF_SOURCE"));
}
#[test]
fn typed_dependency_case_and_direct_hop_citations_are_preserved() {
    let mut e = fixture();
    e[4]["payload"]["type"] = json!("depends_on");
    let c = contract(&e);
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "covered");
    for hop in out["packet"]["dependency_paths"].as_array().unwrap() {
        assert!(hop["journal_citations"][0]["line"].as_u64().unwrap() > 0);
        assert!(hop["journal_citations"][0]["event"].is_string());
    }
    e.push(entry(
        "edge.retract",
        json!({"from":"claim","to":"rule","type":"DEPENDS_ON"}),
        AT,
    ));
    let c = contract(&e);
    assert_eq!(
        run(&e, &c)["packet"]["evidence"].as_array().unwrap().len(),
        1
    );
}
#[test]
fn changed_source_and_stale_second_hop_are_explicit() {
    let mut e = fixture();
    let c = contract(&e);
    e.push(node("source", "Changed content", AT));
    assert_eq!(
        run(&e, &c)["rejected_alternatives"][0]["reason"],
        "source_changed"
    );
    let mut e = fixture();
    e[2] = node("source", "PRIVATE_LEAF_SOURCE", "2026-01-01T00:00:00Z");
    let mut c = contract(&e);
    c.min_observed_utc = "2026-01-02T00:00:00Z".into();
    assert_eq!(
        run(&e, &c)["rejected_alternatives"][0]["reason"],
        "observation_outside_window"
    );
}
#[test]
fn future_node_updates_are_not_current_evidence() {
    let mut e = fixture();
    let c = contract(&e);
    e.push(node("source", "FUTURE_CONTENT", "2026-01-04T00:00:00Z"));
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "covered");
    assert!(!out.to_string().contains("FUTURE_CONTENT"));
}
#[test]
fn conflicts_are_not_avoided_by_selecting_another_alternative() {
    let mut e = fixture();
    e.push(edge("claim", "noise", "CONTRADICTS"));
    let mut c = contract(&e);
    c.facts[0]
        .alternatives
        .push(BTreeSet::from(["noise".into()]));
    assert_eq!(run(&e, &c)["resolution"], "conflict");
    e.push(entry(
        "edge.retract",
        json!({"from":"claim","to":"noise","type":"CONTRADICTS"}),
        AT,
    ));
    let c = contract(&e);
    assert_eq!(run(&e, &c)["resolution"], "covered");
}
#[test]
fn cycles_hop_limits_and_missing_provenance_are_not_success() {
    let mut e = fixture();
    e.push(edge("source", "claim", "DEPENDS_ON"));
    let c = contract(&e);
    assert_eq!(run(&e, &c)["resolution"], "covered");
    let mut c = contract(&e);
    c.max_hops = 1;
    assert_eq!(
        run(&e, &c)["rejected_alternatives"][0]["reason"],
        "hop_limit"
    );
    e[4]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("provenance");
    let c = contract(&e);
    assert_eq!(
        run(&e, &c)["rejected_alternatives"][0]["reason"],
        "dependency_provenance_missing"
    );
}
#[test]
fn exact_packet_budget_and_search_exhaustion_remain_distinct() {
    let e = fixture();
    let mut c = contract(&e);
    let first = run(&e, &c);
    let n = first["packet_tokens"].as_u64().unwrap();
    c.budget_tokens = n;
    assert_eq!(run(&e, &c)["resolution"], "covered");
    c.budget_tokens = n - 1;
    assert_eq!(run(&e, &c)["resolution"], "budget_insufficient");
    c.budget_tokens = 10_000;
    c.search_limit = 1;
    assert_eq!(run(&e, &c)["resolution"], "search_limit_unknown");
}
#[test]
fn shared_alternative_minimizes_rendered_packet_not_individual_sources() {
    let e = vec![
        node("claim", &"long source ".repeat(500), AT),
        node("other", &"other long source ".repeat(500), AT),
        node("shared", "Compact joint evidence", AT),
    ];
    let mut c = contract(&e);
    c.facts = vec![
        Requirement {
            id: "a".into(),
            description: None,
            alternatives: vec![
                BTreeSet::from(["claim".into()]),
                BTreeSet::from(["shared".into()]),
            ],
        },
        Requirement {
            id: "b".into(),
            description: None,
            alternatives: vec![
                BTreeSet::from(["other".into()]),
                BTreeSet::from(["shared".into()]),
            ],
        },
    ];
    let out = run(&e, &c);
    assert_eq!(out["resolution"], "covered");
    assert_eq!(out["packet"]["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(out["packet"]["evidence"][0]["id"], "shared");
}
#[test]
fn malformed_input_is_not_silently_skipped_or_repaired() {
    let e = fixture();
    let c = contract(&e);
    let mut raw = bytes(&e);
    raw.extend_from_slice(b"\n{broken");
    assert!(evaluate(&raw, &c, &c.question).is_err());
    assert!(evaluate(&bytes(&e), &c, "different question").is_err());
    for bad in [
        "2026-02-30T00:00:00Z",
        "2026-01-03T00:00:00+08:00",
        "2026-01-03T25:00:00Z",
    ] {
        assert!(!timestamp(bad));
    }
}

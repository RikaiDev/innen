use super::proposal::citations;
use super::*;
use crate::evidence_closure::{self, Contract, Requirement};
use crate::ids::sha256_hex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[test]
fn evidence_ids_resolve_wrapped_text_and_bind_context_without_model_hash_copying() {
    let (raw, c, _) = fixture();
    let context = id_context(&raw, &c, &c.question).unwrap();
    let draft = IdDraft {
        goal: "Investigate".into(),
        disposition: Disposition::ProceedWithInvestigation,
        requirements: vec![IdFact {
            id: "r1".into(),
            text: "Preserve the failure".into(),
            evidence_ids: vec!["E1".into()],
        }],
        unknowns: vec![],
    };
    let envelope = DraftEnvelope {
        context_sha256: context["context_sha256"].as_str().unwrap().into(),
        draft,
    };
    let p = bind_draft(&raw, &c, &c.question, envelope).unwrap();
    let out = evaluate(&raw, &c, &c.question, &p, None).unwrap();
    assert_eq!(out["resolution"], "source_validated_review_required");
    assert_eq!(
        out["requirements"][0]["citations"][0]["source"],
        "source:one"
    );
    assert_eq!(out["requirements"][0]["citations"][0]["start_byte"], 0);
    assert!(out["requirements"][0]["citations"][0]
        .get("quote")
        .is_none());
    assert_eq!(
        p.requirements[0].alternatives,
        vec![BTreeSet::from(["source:one".into()])]
    );
    let mut stale = p;
    stale.citation_context_sha256 = Some("wrong".into());
    assert!(evaluate(&raw, &c, &c.question, &stale, None).is_err());
}
#[test]
fn invented_or_mixed_evidence_ids_are_rejected() {
    let (raw, c, _) = fixture();
    let base = evidence_closure::proposal_context(&raw, &c, &c.question).unwrap();
    for id in ["E0", "E999", "E01", "other:E1"] {
        assert!(citations(
            &base,
            &[Citation {
                source: String::new(),
                pointer: String::new(),
                quote: String::new(),
                evidence_id: Some(id.into())
            }]
        )
        .is_err());
    }
    assert!(citations(
        &base,
        &[Citation {
            source: "source:one".into(),
            pointer: String::new(),
            quote: String::new(),
            evidence_id: Some("E1".into())
        }]
    )
    .is_err());
}
#[test]
fn overlapping_quotes_and_control_plane_citations_are_rejected() {
    let mut context = json!({"context":{"sources":[{"id":"s","node":{"body":"哈哈哈"},"node_sha256":"fixture"}]}});
    let cite = Citation {
        source: "s".into(),
        pointer: "/body".into(),
        quote: "哈哈".into(),
        evidence_id: None,
    };
    assert!(citations(&context, std::slice::from_ref(&cite)).is_err());
    context["context"]["sources"][0]["node"]["channel"] = json!("control");
    assert!(citations(
        &context,
        &[Citation {
            quote: "哈哈哈".into(),
            ..cite
        }]
    )
    .is_err());
}
fn fixture() -> (Vec<u8>, Contract, Proposal) {
    let at = "2026-01-03T00:00:00Z";
    let n = json!({"id":"source:one","type":"Decision","label":"Build report","body":"Build failed. 不得發布。","provenance":"fixture:source","observed_utc":at});
    let raw = json!({"id":"event:one","op":"node.upsert","payload":n,"observed_utc":at})
        .to_string()
        .into_bytes();
    let c:Contract=serde_json::from_value(json!({"version":1,"question":"Can we publish?","scope_receipt":"fixture:scope","scope_nodes":["source:one"],"as_of":at,"min_observed_utc":at,
        "sources":{"source:one":{"node_sha256":sha256_hex(n.to_string().as_bytes())}},"facts":[],"budget_tokens":5000,"max_hops":2,"search_limit":100})).unwrap();
    let context = evidence_closure::proposal_context(&raw, &c, &c.question).unwrap();
    let p:Proposal=serde_json::from_value(json!({"context_sha256":context["context_sha256"],"question":c.question,"goal":"Investigate the failed build","disposition":"proceed_with_investigation",
        "requirements":[{"id":"failure","text":"Preserve the reported failure and publication restriction","citations":[{"source":"source:one","pointer":"/body","quote":"不得發布。"}],"alternatives":[["source:one"]]}],"unknowns":[]})).unwrap();
    (raw, c, p)
}
fn review(out: &Value, accepted: bool) -> Review {
    let citation = &out["requirements"][0]["citations"][0];
    let span = ReviewSpan {
        source: citation["source"].as_str().unwrap().into(),
        pointer: citation["pointer"].as_str().unwrap().into(),
        start_byte: citation["start_byte"].as_u64().unwrap() as usize,
        end_byte: citation["end_byte"].as_u64().unwrap() as usize,
        node_sha256: citation["node_sha256"].as_str().unwrap().into(),
    };
    let (missing_fact, supports, refutes) = match out["disposition"].as_str().unwrap() {
        "needs_more_evidence" => (
            Some("Evidence needed for the proposed premise".into()),
            vec![],
            vec![],
        ),
        "conflicting_evidence" => (None, vec![], vec![span]),
        _ => (None, vec![span], vec![]),
    };
    Review {
        context_sha256: out["context_sha256"].as_str().unwrap().into(),
        proposal_sha256: out["proposal_sha256"].as_str().unwrap().into(),
        accepted,
        reviewer: "fixture-controller".into(),
        reason: "Compared proposal against source and task conditions".into(),
        premises: if accepted {
            vec![ReviewPremise {
                id: "reviewed-premise".into(),
                claim: "The proposal disposition follows the inspected source".into(),
                missing_fact,
                supports,
                refutes,
            }]
        } else {
            Vec::new()
        },
    }
}
#[test]
fn valid_quotes_require_review_then_compile_without_widening_scope() {
    let (raw, c, p) = fixture();
    let out = evaluate(&raw, &c, &c.question, &p, None).unwrap();
    assert_eq!(out["resolution"], "source_validated_review_required");
    assert!(out["closure"].is_null());
    assert_eq!(
        out["candidate_contract"]["scope_nodes"],
        json!(["source:one"])
    );
    assert_eq!(out["candidate_contract"]["budget_tokens"], 5000);
    let accepted = evaluate(&raw, &c, &c.question, &p, Some(&review(&out, true))).unwrap();
    assert_eq!(accepted["resolution"], "covered");
    assert!(accepted["closure"]["packet"]["facts"][0]["description"].is_string());
    assert!(
        out["requirements"][0]["citations"][0]["start_byte"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[test]
fn invalid_quotes_scopes_and_contexts_reject() {
    let (raw, c, mut p) = fixture();
    p.requirements[0].citations[0].quote = "可以發布".into();
    assert!(evaluate(&raw, &c, &c.question, &p, None).is_err());
    let (raw, c, mut p) = fixture();
    p.requirements[0].alternatives = vec![BTreeSet::from(["outside".into()])];
    assert!(evaluate(&raw, &c, &c.question, &p, None).is_err());
    let (raw, c, mut p) = fixture();
    p.context_sha256 = "wrong".into();
    assert!(evaluate(&raw, &c, &c.question, &p, None).is_err());
}
#[test]
fn omitted_negation_is_not_falsely_certified_and_stale_review_cannot_accept_it() {
    let (raw, c, mut p) = fixture();
    let original = evaluate(&raw, &c, &c.question, &p, None).unwrap();
    let r = review(&original, true);
    p.requirements[0].text = "Publishing is allowed".into();
    let changed = evaluate(&raw, &c, &c.question, &p, None).unwrap();
    assert_eq!(changed["resolution"], "source_validated_review_required");
    assert!(evaluate(&raw, &c, &c.question, &p, Some(&r)).is_err());
    assert_eq!(
        evaluate(&raw, &c, &c.question, &p, Some(&review(&changed, false))).unwrap()["resolution"],
        "review_rejected"
    );
}
#[test]
fn reviewed_missing_or_conflicting_sources_never_produce_execution_packet() {
    for disposition in [
        Disposition::NeedsMoreEvidence,
        Disposition::ConflictingEvidence,
    ] {
        let (raw, c, mut p) = fixture();
        p.disposition = disposition;
        let out = evaluate(&raw, &c, &c.question, &p, None).unwrap();
        let out = evaluate(&raw, &c, &c.question, &p, Some(&review(&out, true))).unwrap();
        assert!(out["closure"].is_null());
        assert_ne!(out["resolution"], "covered");
    }
}
#[test]
fn historical_missing_cases_cannot_be_accepted_as_conflicts() {
    for (id, missing) in [
        (
            "compression-default",
            "No reviewed fact establishes that the codec is ready to become the default",
        ),
        (
            "approved-brand-master",
            "No reviewed fact identifies an approved editable brand master",
        ),
    ] {
        let (raw, c, mut p) = fixture();
        p.disposition = Disposition::ConflictingEvidence;
        let out = evaluate(&raw, &c, &c.question, &p, None).unwrap();
        let mut r = review(&out, true);
        r.premises[0].id = id.into();
        r.premises[0].refutes.clear();
        r.premises[0].missing_fact = Some(missing.into());
        assert_eq!(
            evaluate(&raw, &c, &c.question, &p, Some(&r)).unwrap_err(),
            "proposal disposition is not supported by reviewed premise evidence"
        );
    }
}
#[test]
fn counterevidence_spans_are_versioned_and_utf8_bounded() {
    let (raw, c, mut p) = fixture();
    p.disposition = Disposition::ConflictingEvidence;
    let out = evaluate(&raw, &c, &c.question, &p, None).unwrap();
    let mut r = review(&out, true);
    r.premises[0].refutes[0].node_sha256 = "forged".into();
    assert_eq!(
        evaluate(&raw, &c, &c.question, &p, Some(&r)).unwrap_err(),
        "review span source version is invalid"
    );
    let mut r = review(&out, true);
    r.premises[0].refutes[0].start_byte = 15;
    assert_eq!(
        evaluate(&raw, &c, &c.question, &p, Some(&r)).unwrap_err(),
        "review span byte range is invalid or empty"
    );
}
#[test]
fn model_cannot_supply_scope_or_overwrite_existing_requirements() {
    let (raw, mut c, p) = fixture();
    let mut v = serde_json::to_value(&p).unwrap();
    v["scope_nodes"] = json!(["outside"]);
    assert!(serde_json::from_value::<Proposal>(v).is_err());
    c.facts.push(Requirement {
        id: "locked".into(),
        description: None,
        alternatives: vec![BTreeSet::from(["source:one".into()])],
    });
    assert!(evidence_closure::proposal_context(&raw, &c, &c.question).is_err());
}

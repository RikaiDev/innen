//! External-model task proposals remain untrusted until source and controller review.
//! Citation matching is necessary, not proof of entailment or completeness.
use crate::evidence_closure::{self, Contract, Requirement};
use crate::evidence_state::{self, Premise, Readiness};
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pointer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
}

/// IDs are local to the hash-bound source context, never global identities.
pub fn id_context(raw: &[u8], scope: &Contract, q: &str) -> Result<Value, String> {
    let base = evidence_closure::proposal_context(raw, scope, q)?;
    let mut model = base["context"].clone();
    let mut catalog = Vec::new();
    let mut allowed = Vec::new();
    for (i, source) in model["sources"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let id = format!("E{}", i + 1);
        if let Some(body) = source["node"]["body"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        {
            catalog.push(json!({"evidence_id":id,"source":source["id"],"pointer":"/body","start_byte":0,"end_byte":body.len(),"node_sha256":source["node_sha256"]}));
            if source["node"]["channel"] != "control" {
                allowed.push(id.clone());
            }
            source["evidence_id"] = json!(id);
        }
    }
    if allowed.is_empty() {
        return Err("no business evidence IDs available".into());
    }
    let id_hash=sha256_hex(json!({"format":"innen.body-evidence-ids.v1","source_context_sha256":base["context_sha256"],"catalog":catalog}).to_string().as_bytes());
    Ok(
        json!({"resolution":"proposal_context_prepared","context_sha256":id_hash,"source_context_sha256":base["context_sha256"],"model_context":model,
        "evidence_catalog":catalog,"output_schema":draft_schema(&allowed),
        "id_scope":"Whole body spans in this exact context. No model-authored quotes, offsets, context hash or question are needed in an ID draft."}),
    )
}

fn draft_schema(ids: &[String]) -> Value {
    let refs =
        json!({"type":"array","items":{"type":"string","enum":ids},"minItems":1,"maxItems":16});
    let statement = json!({"type":"object","additionalProperties":false,"properties":{"text":{"type":"string"},"evidence_ids":refs},"required":["text","evidence_ids"]});
    let requirement = json!({"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"},"text":{"type":"string"},"evidence_ids":refs},"required":["id","text","evidence_ids"]});
    json!({"type":"object","additionalProperties":false,"properties":{
        "goal":{"type":"string"},"disposition":{"type":"string","enum":["proceed_with_investigation","needs_more_evidence","conflicting_evidence"]},
        "requirements":{"type":"array","items":requirement,"minItems":1,"maxItems":3},
        "unknowns":{"type":"array","items":statement,"maxItems":2}},"required":["goal","disposition","requirements","unknowns"]})
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdFact {
    pub id: String,
    pub text: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdStatement {
    pub text: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdDraft {
    pub goal: String,
    pub disposition: Disposition,
    pub requirements: Vec<IdFact>,
    pub unknowns: Vec<IdStatement>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftEnvelope {
    pub context_sha256: String,
    pub draft: IdDraft,
}

pub fn bind_draft(
    raw: &[u8],
    scope: &Contract,
    q: &str,
    envelope: DraftEnvelope,
) -> Result<Proposal, String> {
    let context = id_context(raw, scope, q)?;
    if envelope.context_sha256 != context["context_sha256"] {
        return Err("draft envelope does not match its original context".into());
    }
    let draft = envelope.draft;
    let make = |ids: Vec<String>| -> Result<Vec<Citation>, String> {
        if ids.is_empty() || ids.len() > 16 {
            return Err("draft requires bounded evidence IDs".into());
        }
        Ok(ids
            .into_iter()
            .map(|id| Citation {
                source: String::new(),
                pointer: String::new(),
                quote: String::new(),
                evidence_id: Some(id),
            })
            .collect())
    };
    let base = evidence_closure::proposal_context(raw, scope, q)?;
    let mut requirements = Vec::new();
    for r in draft.requirements {
        let cited = make(r.evidence_ids)?;
        let resolved = citations(&base, &cited)?;
        let sources = resolved
            .iter()
            .map(|c| c["source"].as_str().unwrap().to_owned())
            .collect();
        requirements.push(ProposedFact {
            id: r.id,
            text: r.text,
            citations: cited,
            alternatives: vec![sources],
        });
    }
    let unknowns = draft
        .unknowns
        .into_iter()
        .map(|u| {
            Ok(Statement {
                text: u.text,
                citations: make(u.evidence_ids)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Proposal {
        context_sha256: context["source_context_sha256"].as_str().unwrap().into(),
        citation_context_sha256: Some(context["context_sha256"].as_str().unwrap().into()),
        question: q.into(),
        goal: draft.goal,
        disposition: draft.disposition,
        requirements,
        unknowns,
    })
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub text: String,
    pub citations: Vec<Citation>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedFact {
    pub id: String,
    pub text: String,
    pub citations: Vec<Citation>,
    pub alternatives: Vec<BTreeSet<String>>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    ProceedWithInvestigation,
    NeedsMoreEvidence,
    ConflictingEvidence,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub context_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_context_sha256: Option<String>,
    pub question: String,
    pub goal: String,
    pub disposition: Disposition,
    pub requirements: Vec<ProposedFact>,
    pub unknowns: Vec<Statement>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub context_sha256: String,
    pub proposal_sha256: String,
    pub accepted: bool,
    pub reviewer: String,
    pub reason: String,
    #[serde(default)]
    pub premises: Vec<ReviewPremise>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPremise {
    pub id: String,
    pub claim: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_fact: Option<String>,
    #[serde(default)]
    pub supports: Vec<ReviewSpan>,
    #[serde(default)]
    pub refutes: Vec<ReviewSpan>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSpan {
    pub source: String,
    pub pointer: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub node_sha256: String,
}

fn review_schema() -> Value {
    let span = json!({"type":"object","additionalProperties":false,"properties":{
        "source":{"type":"string"},"pointer":{"type":"string"},
        "start_byte":{"type":"integer","minimum":0},"end_byte":{"type":"integer","minimum":1},
        "node_sha256":{"type":"string"}},
        "required":["source","pointer","start_byte","end_byte","node_sha256"]});
    let premise = json!({"type":"object","additionalProperties":false,"properties":{
        "id":{"type":"string"},"claim":{"type":"string"},"missing_fact":{"type":"string"},
        "supports":{"type":"array","items":span,"maxItems":16},
        "refutes":{"type":"array","items":span,"maxItems":16}},
        "required":["id","claim","supports","refutes"]});
    json!({"type":"object","additionalProperties":false,"properties":{
        "context_sha256":{"type":"string"},"proposal_sha256":{"type":"string"},
        "accepted":{"type":"boolean"},"reviewer":{"type":"string"},"reason":{"type":"string"},
        "premises":{"type":"array","items":premise,"minItems":1,"maxItems":16}},
        "required":["context_sha256","proposal_sha256","accepted","reviewer","reason","premises"],
        "note":"Each atomic premise is either supported by exact spans, refuted by exact counterevidence spans, or names one missing_fact. Accepted reviews must match the proposal disposition."})
}

fn review_span(context: &Value, span: &ReviewSpan) -> Result<String, String> {
    let node = context["context"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == span.source)
        .ok_or("review span outside inspected source scope")?;
    if node["node"]["channel"] == "control" || node["node_sha256"] != span.node_sha256 {
        return Err("review span source version is invalid".into());
    }
    let value = node["node"]
        .pointer(&span.pointer)
        .and_then(Value::as_str)
        .ok_or("review span pointer is not a source string")?;
    if span.start_byte >= span.end_byte
        || span.end_byte > value.len()
        || !value.is_char_boundary(span.start_byte)
        || !value.is_char_boundary(span.end_byte)
        || value[span.start_byte..span.end_byte].trim().is_empty()
    {
        return Err("review span byte range is invalid or empty".into());
    }
    Ok(sha256_hex(
        serde_json::to_vec(&json!({
            "source": span.source,
            "pointer": span.pointer,
            "start_byte": span.start_byte,
            "end_byte": span.end_byte,
            "node_sha256": span.node_sha256,
        }))
        .map_err(|e| e.to_string())?
        .as_slice(),
    ))
}

fn review_readiness(context: &Value, review: &Review) -> Result<Readiness, String> {
    if review.premises.is_empty() || review.premises.len() > 16 {
        return Err("accepted review requires bounded explicit premises".into());
    }
    let mut admitted = BTreeSet::new();
    let mut premises = Vec::new();
    for item in &review.premises {
        if item.claim.trim().is_empty() {
            return Err("review premise claim is empty".into());
        }
        if item.supports.len() > 16 || item.refutes.len() > 16 {
            return Err("review premise evidence exceeds bounds".into());
        }
        let supports = item
            .supports
            .iter()
            .map(|s| review_span(context, s))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let refutes = item
            .refutes
            .iter()
            .map(|s| review_span(context, s))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if supports.len() != item.supports.len() || refutes.len() != item.refutes.len() {
            return Err("review premise contains duplicate evidence spans".into());
        }
        admitted.extend(supports.iter().cloned());
        admitted.extend(refutes.iter().cloned());
        premises.push(Premise {
            id: item.id.clone(),
            missing_fact: item.missing_fact.clone(),
            supports,
            refutes,
        });
    }
    evidence_state::assess(&premises, &admitted)
}

fn citations(context: &Value, cites: &[Citation]) -> Result<Vec<Value>, String> {
    if cites.is_empty() || cites.len() > 16 {
        return Err("each assertion requires bounded citations".into());
    }
    let mut out = Vec::new();
    for cite in cites {
        if let Some(id) = &cite.evidence_id {
            if !cite.source.is_empty() || !cite.pointer.is_empty() || !cite.quote.is_empty() {
                return Err("mixed ID and quote citation is ambiguous".into());
            }
            let index = id
                .strip_prefix('E')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| *n > 0)
                .ok_or("invalid evidence ID")?;
            if id != &format!("E{index}") {
                return Err("noncanonical evidence ID".into());
            }
            let node = context["context"]["sources"]
                .as_array()
                .unwrap()
                .get(index - 1)
                .ok_or("evidence ID outside context")?;
            if node["node"]["channel"] == "control" {
                return Err("control-plane events cannot support business requirements".into());
            }
            let body = node["node"]["body"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("evidence ID has no body")?;
            out.push(json!({"evidence_id":id,"source":node["id"],"pointer":"/body","start_byte":0,"end_byte":body.len(),"node_sha256":node["node_sha256"]}));
            continue;
        }
        if cite.quote.trim().is_empty() || cite.quote.len() > 8192 {
            return Err("citation quote empty or too long".into());
        }
        let node = context["context"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == cite.source)
            .ok_or("citation outside inspected source scope")?;
        if node["node"]["channel"] == "control" {
            return Err("control-plane events cannot support business requirements".into());
        }
        let value = node["node"]
            .pointer(&cite.pointer)
            .and_then(Value::as_str)
            .ok_or("citation pointer is not a source string")?;
        let start = value.find(&cite.quote).ok_or("quote absent from source")?;
        let next = start
            + value[start..]
                .chars()
                .next()
                .ok_or("empty citation")?
                .len_utf8();
        if value[next..].contains(&cite.quote) {
            return Err("quote absent or ambiguous; provide a unique exact quote".into());
        }
        out.push(json!({"source":cite.source,"pointer":cite.pointer,"quote":cite.quote,
            "start_byte":start,"end_byte":start+cite.quote.len(),"node_sha256":node["node_sha256"]}));
    }
    Ok(out)
}

pub fn evaluate(
    raw: &[u8],
    scope: &Contract,
    q: &str,
    p: &Proposal,
    review: Option<&Review>,
) -> Result<Value, String> {
    let context = evidence_closure::proposal_context(raw, scope, q)?;
    if p.context_sha256 != context["context_sha256"] || p.question != q {
        return Err("proposal context/question mismatch".into());
    }
    let has_ids = p
        .requirements
        .iter()
        .flat_map(|r| &r.citations)
        .chain(p.unknowns.iter().flat_map(|u| &u.citations))
        .any(|c| c.evidence_id.is_some());
    if has_ids {
        let id_context = id_context(raw, scope, q)?;
        if p.citation_context_sha256.as_deref() != id_context["context_sha256"].as_str() {
            return Err("evidence IDs require their exact versioned catalog digest".into());
        }
    }
    if p.goal.trim().is_empty()
        || p.goal.len() > 4096
        || p.requirements.is_empty()
        || p.requirements.len() > 16
        || p.unknowns.len() > 16
    {
        return Err("proposal goal/requirements/unknowns exceed bounds".into());
    }
    let hash = sha256_hex(serde_json::to_vec(p).map_err(|e| e.to_string())?.as_slice());
    let mut ids = BTreeSet::new();
    let mut checked = Vec::new();
    let mut facts = Vec::new();
    for r in &p.requirements {
        if r.id.trim().is_empty()
            || !ids.insert(&r.id)
            || r.text.trim().is_empty()
            || r.text.len() > 8192
        {
            return Err("requirement IDs/text invalid".into());
        }
        let evidence = citations(&context, &r.citations)?;
        let cited: BTreeSet<_> = evidence
            .iter()
            .map(|c| c["source"].as_str().unwrap().to_owned())
            .collect();
        if r.alternatives.is_empty()
            || r.alternatives.len() > 16
            || r.alternatives
                .iter()
                .any(|a| a.is_empty() || !a.is_subset(&cited))
        {
            return Err("evidence alternatives must use cited inspected sources only".into());
        }
        facts.push(Requirement {
            id: r.id.clone(),
            description: Some(r.text.clone()),
            alternatives: r.alternatives.clone(),
        });
        checked.push(json!({"id":r.id,"text":r.text,"citations":evidence}));
    }
    let mut unknowns = Vec::new();
    for u in &p.unknowns {
        if u.text.trim().is_empty() || u.text.len() > 8192 {
            return Err("unknown statement text invalid".into());
        }
        unknowns.push(json!({"text":u.text,"citations":citations(&context,&u.citations)?}));
    }
    let mut candidate = scope.clone();
    candidate.facts = facts;
    let mut out = json!({"resolution":"source_validated_review_required","context_sha256":p.context_sha256,
        "proposal_sha256":hash,"goal":p.goal,"disposition":p.disposition,"requirements":checked,"unknowns":unknowns,
        "candidate_contract":candidate,"review_schema":review_schema(),"closure":null,
        "limitations":"Quote/hash checks do not prove semantics, missing constraints, or authorization. A review receipt is a caller assertion, not authenticated reviewer identity. No canonical graph writes occur."});
    let Some(review) = review else {
        return Ok(out);
    };
    if review.context_sha256 != p.context_sha256
        || review.proposal_sha256 != hash
        || review.reviewer.trim().is_empty()
        || review.reason.trim().is_empty()
    {
        return Err(
            "review must bind the exact context and proposal hashes and name its basis".into(),
        );
    }
    out["review"] = json!(review);
    if !review.accepted {
        out["resolution"] = json!("review_rejected");
        return Ok(out);
    }
    let readiness = review_readiness(&context, review)?;
    let disposition_matches = matches!(
        (&p.disposition, &readiness),
        (
            Disposition::ProceedWithInvestigation,
            Readiness::SupportedInvestigation
        ) | (Disposition::NeedsMoreEvidence, Readiness::NeedsMoreEvidence)
            | (
                Disposition::ConflictingEvidence,
                Readiness::ConflictingEvidence
            )
    );
    if !disposition_matches {
        return Err("proposal disposition is not supported by reviewed premise evidence".into());
    }
    match p.disposition {
        Disposition::NeedsMoreEvidence => out["resolution"] = json!("reviewed_needs_evidence"),
        Disposition::ConflictingEvidence => out["resolution"] = json!("reviewed_conflict"),
        Disposition::ProceedWithInvestigation => {
            let closure = evidence_closure::evaluate(raw, &candidate, q)?;
            out["resolution"] = closure["resolution"].clone();
            out["closure"] = closure;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            evaluate(&raw, &c, &c.question, &p, Some(&review(&changed, false))).unwrap()
                ["resolution"],
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
}

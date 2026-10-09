use super::id_context::id_context;
use super::proposal::citations;
use super::review::{review_readiness, review_schema};
use super::{Disposition, Proposal, Review};
use crate::evidence_closure::{self, Contract, Requirement};
use crate::evidence_state::Readiness;
use crate::ids::sha256_hex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

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

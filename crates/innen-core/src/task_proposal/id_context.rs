use super::proposal::citations;
use super::{Citation, Disposition, Proposal, ProposedFact, Statement};
use crate::evidence_closure::{self, Contract};
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

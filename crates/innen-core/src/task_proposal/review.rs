use crate::evidence_state::{self, Premise, Readiness};
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

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

pub(super) fn review_schema() -> Value {
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

pub(super) fn review_readiness(context: &Value, review: &Review) -> Result<Readiness, String> {
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

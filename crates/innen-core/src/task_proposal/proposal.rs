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

pub(super) fn citations(context: &Value, cites: &[Citation]) -> Result<Vec<Value>, String> {
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

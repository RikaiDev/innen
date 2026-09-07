//! Model-authored semantic proposals grounded in an immutable AGY source.
//! Mechanical checks establish source integrity/attribution, NOT entailment.
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    UserRequirement,
    AssistantClaim,
    ToolReport,
    Interpretation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Affirmed,
    Negated,
    Unresolved,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Citation {
    pub event: u64,
    /// JSON pointer to a string field in the original event.
    pub pointer: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub quote: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Assertion {
    pub id: String,
    pub basis: Basis,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub polarity: Polarity,
    pub scope: String,
    pub time_scope: String,
    pub qualifiers: Vec<String>,
    pub citations: Vec<Citation>,
    pub supersedes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TailCheck {
    pub after_user_event: u64,
    pub through_event: u64,
}

#[derive(Debug, Deserialize)]
pub struct Proposal {
    pub source_sha256: String,
    pub assertions: Vec<Assertion>,
    pub no_assistant_text_after: Option<TailCheck>,
}

#[derive(Debug, Serialize)]
pub struct ReviewPacket {
    pub status: &'static str,
    pub source_sha256: String,
    pub assertions: Vec<Assertion>,
    pub cited_truncated_events: BTreeSet<u64>,
    pub tail_check_passed: Option<bool>,
    pub limitations: Vec<&'static str>,
}

pub fn validate_source(raw: &[u8], proposal: Proposal) -> Result<ReviewPacket, String> {
    if proposal.assertions.is_empty() {
        return Err("proposal has no assertions".into());
    }
    let digest = sha256_hex(raw);
    if digest != proposal.source_sha256 {
        return Err("source hash mismatch".into());
    }
    let source = std::str::from_utf8(raw).map_err(|_| "source is not UTF-8")?;
    let mut events = BTreeMap::new();
    let mut order = Vec::new();
    for line in source.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|_| "invalid source JSON")?;
        let id = event["step_index"]
            .as_u64()
            .ok_or("missing source step_index")?;
        if events.insert(id, event).is_some() {
            return Err("duplicate source step_index".into());
        }
        order.push(id);
    }
    let mut ids = BTreeSet::new();
    let mut cited_truncated_events = BTreeSet::new();
    for assertion in &proposal.assertions {
        if [
            &assertion.id,
            &assertion.subject,
            &assertion.predicate,
            &assertion.object,
            &assertion.scope,
            &assertion.time_scope,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
            || !ids.insert(assertion.id.clone())
        {
            return Err("assertions need unique IDs and explicit semantic fields (use unresolved when unknown)".into());
        }
        if assertion.citations.is_empty() {
            return Err("assertion has no source citation".into());
        }
        let mut basis_source_present = false;
        for citation in &assertion.citations {
            let event = events
                .get(&citation.event)
                .ok_or("citation event missing")?;
            let text = event
                .pointer(&citation.pointer)
                .and_then(Value::as_str)
                .ok_or("citation pointer is not a source string")?;
            if citation.start_byte >= citation.end_byte
                || text.get(citation.start_byte..citation.end_byte) != Some(citation.quote.as_str())
            {
                return Err("citation bytes do not match source (or split UTF-8)".into());
            }
            let kind = event["type"].as_str().unwrap_or("");
            basis_source_present |= match assertion.basis {
                Basis::UserRequirement => kind == "USER_INPUT" && citation.pointer == "/content",
                Basis::AssistantClaim => {
                    kind == "PLANNER_RESPONSE" && citation.pointer == "/content"
                }
                Basis::ToolReport => kind == "GENERIC" && citation.pointer == "/content",
                Basis::Interpretation => true,
            };
            if event["truncated_fields"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                cited_truncated_events.insert(citation.event);
            }
        }
        if !basis_source_present {
            return Err("evidence basis lacks a citation of the required source role".into());
        }
    }
    let edges: BTreeMap<_, _> = proposal
        .assertions
        .iter()
        .map(|a| (a.id.clone(), a.supersedes.clone()))
        .collect();
    for assertion in &proposal.assertions {
        let mut pending = assertion.supersedes.clone();
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == assertion.id {
                return Err("supersedes cycle".into());
            }
            let next = edges.get(&id).ok_or("supersedes target missing")?;
            if seen.insert(id) {
                pending.extend(next.clone());
            }
        }
    }
    let tail_check_passed = if let Some(check) = proposal.no_assistant_text_after {
        if order.last().copied() != Some(check.through_event)
            || check.after_user_event > check.through_event
            || events
                .get(&check.after_user_event)
                .is_none_or(|e| e["type"] != "USER_INPUT")
        {
            return Err(
                "tail check must start at a user event and end at the actual source end".into(),
            );
        }
        let mut expected = check.after_user_event;
        let start = order
            .iter()
            .position(|id| *id == check.after_user_event)
            .ok_or("tail start missing")?;
        for &id in &order[start..] {
            let event = &events[&id];
            if id != expected {
                return Err(
                    "tail has missing or reordered step IDs; absence cannot be certified".into(),
                );
            }
            expected = id.checked_add(1).ok_or("step overflow")?;
            if event["type"] == "PLANNER_RESPONSE" {
                if event["truncated_fields"]
                    .as_array()
                    .is_some_and(|fields| fields.iter().any(|field| field == "content"))
                {
                    return Err("tail has truncated assistant content; absence is unknown".into());
                }
                if event
                    .get("content")
                    .is_some_and(|value| !value.is_null() && !value.is_string())
                {
                    return Err("tail has an unsupported assistant content representation".into());
                }
            }
            if id > check.after_user_event
                && event["type"] == "PLANNER_RESPONSE"
                && event["content"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty())
            {
                return Err("tail contains assistant text".into());
            }
        }
        Some(true)
    } else {
        None
    };
    Ok(ReviewPacket { status: "source_checks_passed_semantics_unreviewed", source_sha256: digest,
        assertions: proposal.assertions, cited_truncated_events, tail_check_passed,
        limitations: vec!["Matching citations do not establish semantic entailment or completeness.", "Supersedes edges remain model interpretations; acyclicity does not validate their meaning.", "Tail absence covers only this complete supplied suffix, not activity outside the export."] })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<u8>, Proposal) {
        let raw = b"{\"step_index\":0,\"type\":\"USER_INPUT\",\"content\":\"Keep necessary exceptions\"}\n".to_vec();
        let proposal = Proposal {
            source_sha256: sha256_hex(&raw),
            assertions: vec![Assertion {
                id: "a".into(),
                basis: Basis::UserRequirement,
                subject: "parameters".into(),
                predicate: "unify".into(),
                object: "configuration".into(),
                polarity: Polarity::Affirmed,
                scope: "all products".into(),
                time_scope: "requested work".into(),
                qualifiers: vec!["necessary exceptions retained".into()],
                citations: vec![Citation {
                    event: 0,
                    pointer: "/content".into(),
                    start_byte: 0,
                    end_byte: 25,
                    quote: "Keep necessary exceptions".into(),
                }],
                supersedes: vec![],
            }],
            no_assistant_text_after: None,
        };
        (raw, proposal)
    }
    #[test]
    fn matching_source_is_not_semantic_approval() {
        let (raw, p) = fixture();
        assert_eq!(
            validate_source(&raw, p).unwrap().status,
            "source_checks_passed_semantics_unreviewed"
        );
    }
    #[test]
    fn out_of_order_prefix_does_not_invalidate_a_complete_ordered_tail() {
        let (original, mut p) = fixture();
        let mut raw =
            b"{\"step_index\":1,\"type\":\"GENERIC\",\"content\":\"early result\"}\n".to_vec();
        raw.extend_from_slice(&original);
        raw.extend_from_slice(b"{\"step_index\":2,\"type\":\"USER_INPUT\",\"content\":\"later question\"}\n{\"step_index\":3,\"type\":\"GENERIC\",\"content\":\"working\"}\n");
        p.source_sha256 = sha256_hex(&raw);
        p.no_assistant_text_after = Some(TailCheck {
            after_user_event: 2,
            through_event: 3,
        });
        assert_eq!(
            validate_source(&raw, p).unwrap().tail_check_passed,
            Some(true)
        );
    }

    #[test]
    fn omitted_truncated_assistant_text_cannot_certify_absence() {
        let (mut raw, mut p) = fixture();
        raw.extend_from_slice(b"{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"truncated_fields\":[\"content\"]}\n");
        p.source_sha256 = sha256_hex(&raw);
        p.no_assistant_text_after = Some(TailCheck {
            after_user_event: 0,
            through_event: 1,
        });
        assert!(validate_source(&raw, p).is_err());
    }

    #[test]
    fn reject_fabricated_quote_or_role_promotion() {
        let (raw, mut p) = fixture();
        p.assertions[0].citations[0].quote = "No exceptions".into();
        assert!(validate_source(&raw, p).is_err());
        let (raw, mut p) = fixture();
        p.assertions[0].basis = Basis::ToolReport;
        assert!(validate_source(&raw, p).is_err());
    }
    #[test]
    fn reject_wrong_source_cycles_and_false_tail_absence() {
        let (raw, mut p) = fixture();
        p.source_sha256 = "wrong".into();
        assert!(validate_source(&raw, p).is_err());
        let (raw, mut p) = fixture();
        p.assertions[0].supersedes.push("a".into());
        assert!(validate_source(&raw, p).is_err());
        let (mut raw, mut p) = fixture();
        raw.extend_from_slice(
            b"{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"content\":\"answered\"}\n",
        );
        p.source_sha256 = sha256_hex(&raw);
        p.no_assistant_text_after = Some(TailCheck {
            after_user_event: 0,
            through_event: 1,
        });
        assert!(validate_source(&raw, p).is_err());
    }
}

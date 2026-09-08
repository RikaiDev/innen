//! Bounded, offline context packet selection.
//!
//! This module never reads a conversation store or infers relevance. Callers
//! provide every candidate and any optional priority; the result records the
//! exact selected source values and hashes so a caller can expand them later.

pub mod history;

use crate::conversation::grammar::{tokens, tokens_text};
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const TOKENIZER: &str = "o200k_base";
const MAX_ITEMS: usize = 256;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Stable caller-owned prompt prefix. Any JSON value is preserved semantically.
    pub stable_prefix: Value,
    /// Caller-owned current task. Any JSON value is accepted intact.
    pub task: Value,
    /// Ordered caller-selected source context.
    pub items: Vec<Item>,
    pub budget_tokens: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub content: Value,
    pub required: bool,
    /// Higher values are selected first among optional items. Ties retain input order.
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Expansion {
    pub id: String,
    pub selected: bool,
    pub content_sha256: String,
    pub reference_tokens: usize,
}

#[derive(Debug, Serialize)]
pub struct Expanded {
    pub id: String,
    pub content_sha256: String,
    pub reference_tokens: usize,
    pub content: Value,
}

#[derive(Debug, Serialize)]
pub struct ExpansionResult {
    pub resolution: &'static str,
    pub source_sha256: String,
    pub reference_tokenizer: &'static str,
    pub items: Vec<Expanded>,
}

#[derive(Debug, Serialize)]
pub struct Prepared {
    pub resolution: &'static str,
    pub source_sha256: String,
    pub reference_tokenizer: &'static str,
    pub budget_tokens: usize,
    pub before_reference_tokens: usize,
    pub after_reference_tokens: usize,
    pub packet: Value,
    pub selected_ids: Vec<String>,
    pub omitted_ids: Vec<String>,
    pub expansions: Vec<Expansion>,
    pub selection_policy: &'static str,
    /// Exact JSON serialization used by `middleware prepare --packet-only`.
    #[serde(skip)]
    pub packet_json: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("middleware request requires nonempty unique item ids")]
    InvalidItems,
    #[error("middleware request has {actual} items, exceeding the {max} item bound")]
    TooManyItems { actual: usize, max: usize },
    #[error("required context needs {required_reference_tokens} reference tokens, exceeding budget {budget_tokens}")]
    RequiredDoesNotFit {
        budget_tokens: usize,
        required_reference_tokens: usize,
        required_ids: Vec<String>,
    },
    #[error("source hash does not match the supplied request")]
    SourceMismatch { expected: String, actual: String },
    #[error("requested item ids are unknown: {ids:?}")]
    UnknownItems { ids: Vec<String> },
    #[error("tokenization failed: {0}")]
    Token(String),
}

pub fn source_sha256(request: &Request) -> String {
    sha256_hex(&serde_json::to_vec(request).expect("middleware request serializes"))
}

fn validate_items(request: &Request) -> Result<(), Error> {
    if request.items.len() > MAX_ITEMS {
        return Err(Error::TooManyItems {
            actual: request.items.len(),
            max: MAX_ITEMS,
        });
    }
    let mut ids = BTreeSet::new();
    if request
        .items
        .iter()
        .any(|item| item.id.is_empty() || !ids.insert(&item.id))
    {
        return Err(Error::InvalidItems);
    }
    Ok(())
}

fn packet_json(request: &Request, selected: &BTreeSet<usize>) -> String {
    #[derive(Serialize)]
    struct Packet<'a> {
        stable_prefix: &'a Value,
        task: &'a Value,
        items: Vec<PacketItem<'a>>,
    }
    #[derive(Serialize)]
    struct PacketItem<'a> {
        id: &'a str,
        content: &'a Value,
    }
    let items = request
        .items
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(_, item)| PacketItem {
            id: &item.id,
            content: &item.content,
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&Packet {
        stable_prefix: &request.stable_prefix,
        task: &request.task,
        items,
    })
    .expect("middleware packet serializes")
}

/// Prepare a packet using only caller supplied order and priority.
///
/// Required items are always included or the result is explicitly unknown.
/// Optional candidates are considered by descending explicit priority, with
/// original input order breaking ties. A candidate is included only if the
/// whole rendered JSON packet remains within the caller's reference budget.
pub fn prepare(request: Request) -> Result<Prepared, Error> {
    validate_items(&request)?;

    let source_sha256 = source_sha256(&request);
    let all = (0..request.items.len()).collect::<BTreeSet<_>>();
    let before_reference_tokens =
        tokens_text(&packet_json(&request, &all)).map_err(Error::Token)?;

    let mut selected = request
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.required.then_some(index))
        .collect::<BTreeSet<_>>();
    let required_reference_tokens =
        tokens_text(&packet_json(&request, &selected)).map_err(Error::Token)?;
    if required_reference_tokens > request.budget_tokens {
        return Err(Error::RequiredDoesNotFit {
            budget_tokens: request.budget_tokens,
            required_reference_tokens,
            required_ids: selected
                .iter()
                .map(|index| request.items[*index].id.clone())
                .collect(),
        });
    }

    let mut optional = request
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.required)
        .collect::<Vec<_>>();
    optional.sort_by_key(|(index, item)| (std::cmp::Reverse(item.priority.unwrap_or(0)), *index));
    for (index, _) in optional {
        let mut candidate = selected.clone();
        candidate.insert(index);
        if tokens_text(&packet_json(&request, &candidate)).map_err(Error::Token)?
            <= request.budget_tokens
        {
            selected = candidate;
        }
    }

    let packet_json = packet_json(&request, &selected);
    let packet = serde_json::from_str(&packet_json).expect("middleware packet parses");
    let after_reference_tokens = tokens_text(&packet_json).map_err(Error::Token)?;
    let selected_ids = selected
        .iter()
        .map(|index| request.items[*index].id.clone())
        .collect::<Vec<_>>();
    let omitted_ids = request
        .items
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected.contains(index))
        .map(|(_, item)| item.id.clone())
        .collect::<Vec<_>>();
    let expansions = request
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            Ok(Expansion {
                id: item.id.clone(),
                selected: selected.contains(&index),
                content_sha256: sha256_hex(
                    &serde_json::to_vec(&item.content).expect("JSON value serializes"),
                ),
                reference_tokens: tokens(&item.content).map_err(Error::Token)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Prepared {
        resolution: "prepared",
        source_sha256,
        reference_tokenizer: TOKENIZER,
        budget_tokens: request.budget_tokens,
        before_reference_tokens,
        after_reference_tokens,
        packet,
        selected_ids,
        omitted_ids,
        expansions,
        selection_policy: "required_then_optional_descending_caller_priority_then_input_order",
        packet_json,
    })
}

/// Return exact caller-provided items only when their request hash still matches.
pub fn expand(
    request: Request,
    expected_source_sha256: &str,
    requested_ids: &[String],
) -> Result<ExpansionResult, Error> {
    validate_items(&request)?;
    let actual = source_sha256(&request);
    if expected_source_sha256 != actual {
        return Err(Error::SourceMismatch {
            expected: expected_source_sha256.to_owned(),
            actual,
        });
    }
    let known = request
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let unknown = requested_ids
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(Error::UnknownItems { ids: unknown });
    }
    let wanted = requested_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let items = request
        .items
        .iter()
        .filter(|item| wanted.contains(item.id.as_str()))
        .map(|item| {
            Ok(Expanded {
                id: item.id.clone(),
                content_sha256: sha256_hex(
                    &serde_json::to_vec(&item.content).expect("JSON value serializes"),
                ),
                reference_tokens: tokens(&item.content).map_err(Error::Token)?,
                content: item.content.clone(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(ExpansionResult {
        resolution: "expanded",
        source_sha256: actual,
        reference_tokenizer: TOKENIZER,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_required_and_uses_explicit_priority() {
        let mut request: Request = serde_json::from_value(json!({
            "stable_prefix":{"system":"fixed"}, "task":["do work"], "budget_tokens":1000,
            "items":[
                {"id":"required","content":{"a":"keep"},"required":true},
                {"id":"low","content":"low","required":false,"priority":1},
                {"id":"high","content":"high","required":false,"priority":9}
            ]
        }))
        .unwrap();
        let required = [0].into_iter().collect::<BTreeSet<_>>();
        let high = [0, 2].into_iter().collect::<BTreeSet<_>>();
        request.budget_tokens = tokens_text(&packet_json(&request, &high)).unwrap();
        assert!(tokens_text(&packet_json(&request, &required)).unwrap() < request.budget_tokens);
        let output = prepare(request).unwrap();
        assert_eq!(output.selected_ids, ["required", "high"]);
        assert_eq!(output.omitted_ids, ["low"]);
        assert_eq!(output.packet["stable_prefix"], json!({"system":"fixed"}));
        assert_eq!(output.packet["task"], json!(["do work"]));
        assert!(output
            .expansions
            .iter()
            .any(|x| x.id == "low" && !x.selected));
    }

    #[test]
    fn required_over_budget_is_explicit_unknown() {
        let request: Request = serde_json::from_value(json!({
            "stable_prefix":"prefix", "task":"task", "budget_tokens":1,
            "items":[{"id":"must","content":"required source","required":true}]
        }))
        .unwrap();
        assert!(matches!(
            prepare(request),
            Err(Error::RequiredDoesNotFit { .. })
        ));
    }

    #[test]
    fn expansion_requires_matching_source_hash_and_preserves_item_order() {
        let request: Request = serde_json::from_value(json!({
            "stable_prefix":null, "task":{}, "budget_tokens":100,
            "items":[
                {"id":"one","content":{"nested":[1]},"required":false},
                {"id":"two","content":"exact","required":true}
            ]
        }))
        .unwrap();
        let hash = source_sha256(&request);
        let out = expand(request, &hash, &["two".into(), "one".into()]).unwrap();
        assert_eq!(
            out.items.iter().map(|item| &item.id).collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(matches!(
            expand(
                serde_json::from_value(
                    json!({"stable_prefix":null,"task":{},"budget_tokens":100,"items":[]})
                )
                .unwrap(),
                &hash,
                &[]
            ),
            Err(Error::SourceMismatch { .. })
        ));
    }

    #[test]
    fn rejects_more_than_the_bounded_number_of_caller_items() {
        let request: Request = serde_json::from_value(json!({
            "stable_prefix":null, "task":null, "budget_tokens":100,
            "items":(0..=MAX_ITEMS).map(|n| json!({"id":format!("id-{n}"),"content":null,"required":false})).collect::<Vec<_>>(),
        }))
        .unwrap();
        assert!(matches!(prepare(request), Err(Error::TooManyItems { .. })));
    }
}

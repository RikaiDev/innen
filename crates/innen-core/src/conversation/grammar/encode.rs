//! The encode driver: try both page framings and both chunking modes, and keep
//! whichever packet costs the fewest reference tokens.

use super::compress::{inventory, Grammar};
use super::tokens::tokens;
use super::{MAX_BYTES, MAX_RULES};
use crate::ids::sha256_hex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Select by complete reference-token cost, including framing and grammar overhead.
/// Never claim this reference encoding is the provider's billing tokenizer.
pub fn encode(original: &Value, legacy: &Value) -> Result<Value, String> {
    if original.to_string().len() > MAX_BYTES {
        return Err("conversation grammar page exceeds 2 MiB work bound; paginate without dropping source events".into());
    }
    let mut best = original.clone();
    let mut score = tokens(&best)?;
    if tokens(legacy)? < score {
        best = legacy.clone();
        score = tokens(legacy)?;
    }
    let hash = sha256_hex(original.to_string().as_bytes());
    for base in [original, legacy] {
        let mut texts = BTreeMap::new();
        let mut keys = BTreeSet::new();
        inventory(base, &mut texts, &mut keys);
        let mut key = "$seq".to_owned();
        while keys.contains(&key) {
            key.push('_');
        }
        for lines in [true, false] {
            let Some(mut grammar) =
                Grammar::new(texts.iter().map(|(s, n)| (s.clone(), *n)).collect(), lines)
            else {
                continue;
            };
            let mut current = grammar.packet(base, &key, &hash);
            let mut cost = tokens(&current)?;
            for _ in 0..MAX_RULES {
                let mut next = None;
                for pair in grammar.pairs() {
                    let mut trial = grammar.clone();
                    trial.replace(pair);
                    let packet = trial.packet(base, &key, &hash);
                    let n = tokens(&packet)?;
                    if n < cost && next.as_ref().is_none_or(|(_, _, prior)| n < *prior) {
                        next = Some((trial, packet, n));
                    }
                }
                match next {
                    Some((g, p, n)) => {
                        grammar = g;
                        current = p;
                        cost = n;
                    }
                    None => break,
                }
            }
            if cost < score {
                if super::decode::decode(&current)? != *original {
                    return Err("conversation grammar roundtrip mismatch".into());
                }
                best = current;
                score = cost;
            }
        }
    }
    Ok(best)
}

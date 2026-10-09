//! The compression engine: content-defined chunking, weighted pair selection, and
//! packet assembly against a shared dictionary.

use super::MAX_ATOMS;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) enum Symbol {
    Text(String),
    Pair(usize, usize),
}

#[derive(Clone)]
pub(super) struct Grammar {
    symbols: Vec<Symbol>,
    lengths: Vec<usize>,
    texts: Vec<String>,
    weights: Vec<usize>,
    sequences: Vec<Vec<usize>>,
}

pub(super) fn inventory(
    value: &Value,
    texts: &mut BTreeMap<String, usize>,
    keys: &mut BTreeSet<String>,
) {
    match value {
        Value::String(s) if s.len() >= 128 => {
            *texts.entry(s.clone()).or_default() += 1;
        }
        Value::Array(a) => {
            for x in a {
                inventory(x, texts, keys);
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                keys.insert(k.clone());
                inventory(v, texts, keys);
            }
        }
        _ => {}
    }
}

pub(super) fn chunks(text: &str, lines: bool) -> Vec<&str> {
    if lines {
        return text.split_inclusive('\n').collect();
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut hash = 0u64;
    for (i, c) in text.char_indices() {
        // A bounded rolling gear hash chooses boundaries, never content identity.
        hash = hash
            .wrapping_shl(1)
            .wrapping_add((c as u64).wrapping_mul(0x9e3779b185ebca87));
        let end = i + c.len_utf8();
        let len = end - start;
        if len >= 64 && ((hash & 127) == 0 || len >= 384 || c == '\n') {
            out.push(&text[start..end]);
            start = end;
            hash = 0;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

impl Grammar {
    pub(super) fn new(weighted_texts: Vec<(String, usize)>, lines: bool) -> Option<Self> {
        let (texts, weights): (Vec<_>, Vec<_>) = weighted_texts.into_iter().unzip();
        let mut symbols = Vec::new();
        let mut lengths = Vec::new();
        let mut map = BTreeMap::new();
        let mut sequences = Vec::new();
        for text in &texts {
            let mut seq = Vec::new();
            for chunk in chunks(text, lines) {
                let id = *map.entry(chunk.to_owned()).or_insert_with(|| {
                    let id = symbols.len();
                    symbols.push(Symbol::Text(chunk.to_owned()));
                    lengths.push(chunk.len());
                    id
                });
                seq.push(id);
            }
            if symbols.len() > MAX_ATOMS {
                return None;
            }
            sequences.push(seq);
        }
        Some(Self {
            symbols,
            lengths,
            texts,
            weights,
            sequences,
        })
    }
    pub(super) fn pairs(&self) -> Vec<(usize, usize)> {
        let mut counts = BTreeMap::<(usize, usize), usize>::new();
        for (seq, weight) in self.sequences.iter().zip(&self.weights) {
            for p in seq.windows(2) {
                *counts.entry((p[0], p[1])).or_default() += weight;
            }
        }
        let mut ranked: Vec<_> = counts
            .into_iter()
            .filter_map(|(pair, n)| {
                let bytes = self.lengths[pair.0] + self.lengths[pair.1];
                let gain = bytes
                    .saturating_mul(n.saturating_sub(1))
                    .saturating_sub(n * 8 + 32);
                (n >= 2 && gain > 0).then_some((gain, pair))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.into_iter().take(4).map(|(_, pair)| pair).collect()
    }
    pub(super) fn replace(&mut self, pair: (usize, usize)) {
        let id = self.symbols.len();
        self.symbols.push(Symbol::Pair(pair.0, pair.1));
        self.lengths
            .push(self.lengths[pair.0] + self.lengths[pair.1]);
        for seq in &mut self.sequences {
            let mut next = Vec::new();
            let mut i = 0;
            while i < seq.len() {
                if i + 1 < seq.len() && (seq[i], seq[i + 1]) == pair {
                    next.push(id);
                    i += 2;
                } else {
                    next.push(seq[i]);
                    i += 1;
                }
            }
            *seq = next;
        }
    }
    pub(super) fn packet(&self, base: &Value, key: &str, hash: &str) -> Value {
        let mut counts = vec![0usize; self.symbols.len()];
        for (seq, weight) in self.sequences.iter().zip(&self.weights) {
            for &id in seq {
                counts[id] += weight;
            }
        }
        // Count definition dependencies once, not every logical expansion.
        for id in (0..self.symbols.len()).rev() {
            if counts[id] > 0 {
                if let Symbol::Pair(a, b) = self.symbols[id] {
                    counts[a] += 1;
                    counts[b] += 1;
                }
            }
        }
        let selected: Vec<_> = counts
            .iter()
            .enumerate()
            .map(|(i, n)| *n > 1 && self.lengths[i] >= 32)
            .collect();
        let mut definitions = Vec::<Value>::new();
        let mut refs = BTreeMap::new();
        fn push(out: &mut Vec<Value>, value: Value) {
            if let (Some(Value::String(previous)), Value::String(next)) = (out.last_mut(), &value) {
                previous.push_str(next);
            } else {
                out.push(value);
            }
        }
        fn emit(
            id: usize,
            g: &Grammar,
            selected: &[bool],
            defs: &mut Vec<Value>,
            refs: &mut BTreeMap<usize, usize>,
            out: &mut Vec<Value>,
            skip: bool,
        ) {
            if selected[id] && !skip {
                let index = if let Some(&index) = refs.get(&id) {
                    index
                } else {
                    let mut parts = Vec::new();
                    emit(id, g, selected, defs, refs, &mut parts, true);
                    let definition = if parts.len() == 1 && parts[0].is_string() {
                        parts.remove(0)
                    } else {
                        json!(parts)
                    };
                    let index = defs.len();
                    defs.push(definition);
                    refs.insert(id, index);
                    index
                };
                push(out, json!(index));
                return;
            }
            match &g.symbols[id] {
                Symbol::Text(s) => push(out, json!(s)),
                Symbol::Pair(a, b) => {
                    emit(*a, g, selected, defs, refs, out, false);
                    emit(*b, g, selected, defs, refs, out, false);
                }
            }
        }
        let mut replacements = BTreeMap::new();
        for (text, seq) in self.texts.iter().zip(&self.sequences) {
            let mut parts = Vec::new();
            for &id in seq {
                emit(
                    id,
                    self,
                    &selected,
                    &mut definitions,
                    &mut refs,
                    &mut parts,
                    false,
                );
            }
            if parts.iter().any(Value::is_number) {
                replacements.insert(text.clone(), json!({key:parts}));
            }
        }
        fn rewrite(value: &mut Value, replacements: &BTreeMap<String, Value>) {
            match value {
                Value::String(s) => {
                    if let Some(r) = replacements.get(s) {
                        *value = r.clone();
                    }
                }
                Value::Array(a) => {
                    for x in a {
                        rewrite(x, replacements);
                    }
                }
                Value::Object(o) => {
                    for x in o.values_mut() {
                        rewrite(x, replacements);
                    }
                }
                _ => {}
            }
        }
        let mut page = base.clone();
        rewrite(&mut page, &replacements);
        json!({"encoding":super::ENCODING,"guide":super::GUIDE,"join_key":key,"dictionary":definitions,"page":page,
               "source_sha256":hash,"reference_tokenizer":"o200k_base"})
    }
}

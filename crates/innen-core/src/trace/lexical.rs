use super::types::LexDoc;
use std::collections::{BTreeMap, BTreeSet};

fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3134f | 0x3040..=0x30ff | 0xac00..=0xd7af)
}

fn stop(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "a"
            | "an"
            | "and"
            | "or"
            | "to"
            | "of"
            | "in"
            | "is"
            | "for"
            | "with"
            | "的"
            | "了"
            | "是"
            | "我"
            | "你"
            | "請"
            | "之前"
    )
}

fn flush(word: &mut String, out: &mut Vec<String>) {
    if !word.is_empty() {
        let token = std::mem::take(word);
        if !stop(&token) {
            out.push(token);
        }
    }
}

/// Streaming Unicode terms: alphanumeric runs and CJK uni/bigrams.
/// Query and document indexing use the same deterministic representation.
pub(super) fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut previous = None;
    for ch in text.to_lowercase().chars() {
        if cjk(ch) {
            flush(&mut word, &mut out);
            let single = ch.to_string();
            if !stop(&single) {
                out.push(single);
            }
            if let Some(prev) = previous {
                let pair = format!("{prev}{ch}");
                if !stop(&pair) {
                    out.push(pair);
                }
            }
            previous = Some(ch);
        } else {
            previous = None;
            if ch.is_alphanumeric() {
                word.push(ch);
            } else {
                flush(&mut word, &mut out);
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

pub(super) fn lex(id: String, text: &str) -> LexDoc {
    let words = tokens(text);
    let mut tf = BTreeMap::new();
    for word in &words {
        *tf.entry(word.clone()).or_insert(0) += 1;
    }
    LexDoc {
        id,
        tf,
        len: words.len(),
    }
}
pub(super) fn bm25(docs: &[LexDoc], query: &[String]) -> Vec<(usize, f64)> {
    if docs.is_empty() || query.is_empty() {
        return Vec::new();
    }
    let terms: BTreeSet<_> = query.iter().collect();
    let avg = (docs.iter().map(|d| d.len).sum::<usize>() as f64 / docs.len() as f64).max(1.0);
    let df: BTreeMap<_, _> = terms
        .iter()
        .map(|t| {
            (
                *t,
                docs.iter()
                    .filter(|d| d.tf.contains_key(t.as_str()))
                    .count(),
            )
        })
        .collect();
    let mut ranked = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let mut score = 0.0;
        for term in &terms {
            let count = f64::from(*doc.tf.get(term.as_str()).unwrap_or(&0));
            if count == 0.0 {
                continue;
            }
            let frequency = df[term] as f64;
            let idf = (1.0 + (docs.len() as f64 - frequency + 0.5) / (frequency + 0.5)).ln();
            let term_weight = if term.chars().count() == 1 && term.chars().next().is_some_and(cjk) {
                0.2
            } else {
                1.0
            };
            score += term_weight * idf * count * 2.2
                / (count + 1.2 * (0.25 + 0.75 * doc.len as f64 / avg));
        }
        if score > 0.0 {
            ranked.push((i, score));
        }
    }
    ranked.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| docs[a.0].id.cmp(&docs[b.0].id))
    });
    ranked
}

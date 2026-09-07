//! Exact repeated line-prefix factoring inside strings. No line is removed and
//! no semantic equivalence is inferred. Uses the existing literal-fragment wire.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn encode(original: &Value) -> Option<Value> {
    fn visit<'a>(v: &'a Value, texts: &mut Vec<&'a str>, keys: &mut BTreeSet<&'a str>) {
        match v {
            Value::String(s) => texts.push(s),
            Value::Array(items) => {
                for item in items {
                    visit(item, texts, keys);
                }
            }
            Value::Object(object) => {
                for (key, item) in object {
                    keys.insert(key);
                    visit(item, texts, keys);
                }
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    let mut keys = BTreeSet::new();
    visit(original, &mut texts, &mut keys);
    let mut counts = BTreeMap::<&str, usize>::new();
    for text in &texts {
        for line in text.split_inclusive('\n') {
            if line.len() >= 64 {
                *counts.entry(line).or_default() += 1;
            }
        }
    }
    let lines: Vec<_> = counts.keys().copied().collect();
    let mut candidates = BTreeSet::new();
    for (&line, &count) in &counts {
        if count > 1
            && !line
                .trim_matches([' ', '\t', '\r', '\n', '|', '+', '-'])
                .is_empty()
        {
            candidates.insert(line);
        }
    }
    for pair in lines.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut n = a.bytes().zip(b.bytes()).take_while(|(a, b)| a == b).count();
        while !a.is_char_boundary(n) || !b.is_char_boundary(n) {
            n -= 1;
        }
        let prefix = &a[..n];
        if n >= 64
            && !prefix
                .trim_matches([' ', '\t', '\r', '\n', '|', '+', '-'])
                .is_empty()
        {
            candidates.insert(prefix);
        }
    }
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .map(|p| {
            let count: usize = counts
                .iter()
                .filter(|(line, _)| line.starts_with(p))
                .map(|(_, n)| *n)
                .sum();
            (
                (count.saturating_sub(1) * p.len()).saturating_sub(count * 12),
                p,
            )
        })
        .collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    let mut assigned = BTreeMap::<&str, &str>::new();
    for (score, prefix) in ranked {
        if score == 0 {
            continue;
        }
        let matching: Vec<_> = lines
            .iter()
            .copied()
            .filter(|line| !assigned.contains_key(line) && line.starts_with(prefix))
            .collect();
        if matching.iter().map(|line| counts[line]).sum::<usize>() < 2 {
            continue;
        }
        for line in matching {
            assigned.insert(line, prefix);
        }
    }
    let mut fragments = Vec::<String>::new();
    let mut ids = BTreeMap::<String, usize>::new();
    let mut replacements = BTreeMap::<&str, Vec<usize>>::new();
    let unique: BTreeSet<_> = texts.into_iter().collect();
    for text in unique {
        let mut parts = Vec::<String>::new();
        let mut buffer = String::new();
        let mut changed = false;
        for line in text.split_inclusive('\n') {
            if let Some(prefix) = assigned.get(line) {
                if !buffer.is_empty() {
                    parts.push(std::mem::take(&mut buffer));
                }
                parts.push((*prefix).to_owned());
                buffer.push_str(&line[prefix.len()..]);
                changed = true;
            } else {
                buffer.push_str(line);
            }
        }
        if !changed {
            continue;
        }
        if !buffer.is_empty() {
            parts.push(buffer);
        }
        let join = parts
            .into_iter()
            .map(|part| {
                *ids.entry(part.clone()).or_insert_with(|| {
                    fragments.push(part);
                    fragments.len() - 1
                })
            })
            .collect();
        replacements.insert(text, join);
    }
    if replacements.is_empty() {
        return None;
    }
    let mut key = "$join".to_owned();
    while keys.contains(key.as_str()) {
        key.push('_');
    }
    fn replace(v: &mut Value, replacements: &BTreeMap<&str, Vec<usize>>, key: &str) {
        match v {
            Value::String(s) => {
                if let Some(join) = replacements.get(s.as_str()) {
                    *v = json!({key:join});
                }
            }
            Value::Array(items) => {
                for item in items {
                    replace(item, replacements, key);
                }
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    replace(item, replacements, key);
                }
            }
            _ => {}
        }
    }
    let mut page = original.clone();
    replace(&mut page, &replacements, &key);
    let packed =
        json!({"encoding":"innen.edges.v1","join_key":key,"fragments":fragments,"page":page});
    (packed.to_string().len() < original.to_string().len()).then_some(packed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_whole_lines_keep_every_occurrence() {
        let line = format!("FAIL remains evidence: {}\n", "detail ".repeat(30));
        let original = json!({"body":format!("{line}{line}")});
        let packed = encode(&original).unwrap();
        assert_eq!(packed["fragments"].as_array().unwrap().len(), 1);
        assert_eq!(super::super::deltas::decode(&packed).unwrap(), original);
    }
    #[test]
    fn repeated_paths_inside_one_string_restore_and_unused_values_are_not_duplicated() {
        let base = format!("/private/{}", "共同來源/".repeat(20));
        let text = (0..30)
            .map(|i| format!("{base}file-{i}.txt\n"))
            .collect::<String>();
        let unchanged = "unrelated ".repeat(500);
        let original = json!({"paths":text,"unchanged":unchanged,"literal":{"$join":[0]}});
        let packed = encode(&original).unwrap();
        assert_eq!(super::super::deltas::decode(&packed).unwrap(), original);
        assert!(!packed["fragments"]
            .as_array()
            .unwrap()
            .contains(&json!(unchanged)));
        assert_ne!(packed["join_key"], "$join");
        assert!(packed.to_string().len() < original.to_string().len());
    }
}

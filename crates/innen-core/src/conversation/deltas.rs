//! Optional exact shared-edge factoring. Fragments are literal strings and join
//! in order, without byte offsets, patches, inferred edits or reference chains.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn inventory<'a>(value: &'a Value, texts: &mut BTreeSet<&'a str>, keys: &mut BTreeSet<&'a str>) {
    match value {
        Value::String(s) if s.len() >= 256 => {
            texts.insert(s);
        }
        Value::Array(items) => {
            for item in items {
                inventory(item, texts, keys);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                keys.insert(key);
                inventory(item, texts, keys);
            }
        }
        _ => {}
    }
}

fn edges(a: &str, b: &str) -> (usize, usize) {
    let mut prefix = a.bytes().zip(b.bytes()).take_while(|(a, b)| a == b).count();
    while !a.is_char_boundary(prefix) || !b.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = a[prefix..]
        .bytes()
        .rev()
        .zip(b[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !a.is_char_boundary(a.len() - suffix) || !b.is_char_boundary(b.len() - suffix) {
        suffix -= 1;
    }
    (prefix, suffix)
}

pub fn encode(original: Value) -> Value {
    let prefix = super::prefixes::encode(&original);
    let edges = encode_edges(original);
    match prefix {
        Some(prefix) if prefix.to_string().len() < edges.to_string().len() => prefix,
        _ => edges,
    }
}

fn encode_edges(original: Value) -> Value {
    let mut texts = BTreeSet::new();
    let mut keys = BTreeSet::new();
    inventory(&original, &mut texts, &mut keys);
    let texts: Vec<_> = texts.into_iter().collect();
    let mut key = "$join".to_owned();
    while keys.contains(key.as_str()) {
        key.push('_');
    }
    let mut fragments = Vec::<String>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    let mut replacements = BTreeMap::<&str, Vec<usize>>::new();
    let mut i = 0;
    while i + 1 < texts.len() {
        let (a, b) = (texts[i], texts[i + 1]);
        let (prefix, suffix) = edges(a, b);
        if prefix + suffix < 256 {
            i += 1;
            continue;
        }
        for text in [a, b] {
            let mut join = Vec::new();
            for part in [
                &text[..prefix],
                &text[prefix..text.len() - suffix],
                &text[text.len() - suffix..],
            ] {
                if part.is_empty() {
                    continue;
                }
                let index = *indices.entry(part.to_owned()).or_insert_with(|| {
                    fragments.push(part.to_owned());
                    fragments.len() - 1
                });
                join.push(index);
            }
            replacements.insert(text, join);
        }
        i += 2;
    }
    if replacements.is_empty() {
        return original;
    }
    fn replace(value: &mut Value, replacements: &BTreeMap<&str, Vec<usize>>, key: &str) {
        match value {
            Value::String(s) => {
                if let Some(join) = replacements.get(s.as_str()) {
                    *value = json!({key:join});
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
    if packed.to_string().len() < original.to_string().len() {
        packed
    } else {
        original
    }
}

/// Restore the wrapped representation. Other codecs are decoded afterward.
pub fn decode(value: &Value) -> Result<Value, String> {
    if value["encoding"] != "innen.edges.v1" {
        return Ok(value.clone());
    }
    let key = value["join_key"].as_str().ok_or("missing join key")?;
    let fragments = value["fragments"].as_array().ok_or("missing fragments")?;
    if fragments.iter().any(|f| !f.is_string()) {
        return Err("nonliteral fragment".into());
    }
    let mut page = value
        .get("page")
        .filter(|v| v.is_object())
        .ok_or("missing page")?
        .clone();
    fn restore(value: &mut Value, fragments: &[Value], key: &str) -> Result<(), String> {
        match value {
            Value::Object(object) if object.contains_key(key) => {
                if object.len() != 1 {
                    return Err("ambiguous join marker".into());
                }
                let parts = object[key].as_array().ok_or("invalid join")?;
                let mut text = String::new();
                for index in parts {
                    let i = index
                        .as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or("invalid fragment index")?;
                    text.push_str(
                        fragments
                            .get(i)
                            .and_then(Value::as_str)
                            .ok_or("missing fragment")?,
                    );
                }
                *value = Value::String(text);
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    restore(item, fragments, key)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    restore(item, fragments, key)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    restore(&mut page, fragments, key)?;
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_unicode_edges_keep_changes_negation_and_literal_markers() {
        let prefix = "共同條件；\r\n".repeat(80);
        let suffix = "保持順序。\n".repeat(80);
        let original = json!({"before":format!("{prefix}不得發布{suffix}"),"after":format!("{prefix}可以發布{suffix}"),"literal":{"$join":[0]}});
        let packed = encode(original.clone());
        assert_eq!(packed["encoding"], "innen.edges.v1");
        assert_ne!(packed["join_key"], "$join");
        assert_eq!(decode(&packed).unwrap(), original);
        assert_eq!(super::super::compact::decode(&packed).unwrap(), original);
        let mut bad = packed;
        bad["fragments"] = json!([]);
        assert!(decode(&bad).is_err());
    }
    #[test]
    fn unicode_boundaries_overlap_and_small_fallback() {
        assert_eq!(edges("中", "串"), (0, 0));
        assert_eq!(edges("prefix", "prefix-long"), (6, 0));
        let original = json!({"a":"no","b":"yes"});
        assert_eq!(encode(original.clone()), original);
        let shared = "甲".repeat(300);
        let original = json!({"a":shared,"b":format!("{shared}乙")});
        assert_eq!(decode(&encode(original.clone())).unwrap(), original);
    }
}

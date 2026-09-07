//! Reversible page-local factoring. No relevance scoring or semantic merging.
//! Each row is [source_line, layout_index, values_in_field_order]. Shared fields
//! apply to every row of that layout. A separate string table can factor exact
//! repeated long strings, including nested values. Byte-size admission is not
//! a guarantee of tokenizer savings. Neither transform drops occurrences.

use super::Page;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const COMMON_GUIDE: &str = "For innen.strings.v1, replace each singleton object keyed by ref_key in page with texts[index]. For innen.rows.v1, each row is [source line, layout index, values]; its event merges layout.shared with values keyed by layout.fields in order. Preserve row order and all page metadata. Other pages are ordinary JSON.";
const EDGE_GUIDE: &str = "For innen.edges.v1, replace each singleton object keyed by join_key in page with the concatenation of fragments at its listed indices, in order, with no separator. Fragments are literal strings. Then decode the restored page.";
const ATTACHMENT_GUIDE: &str = "innen.attachments.v1 omits source payloads. Decode page, then each attachments entry identifies a record, source line and event JSON pointer. Retrieve using read at offset line minus one with --attachment pointer --expect-sha256 hash. Replace only the listed marker with the returned value. Images and encrypted content have not been interpreted.";

/// Make CLI packets self-describing without an out-of-band model prompt.
/// `guide` is framing metadata, never source content.
pub fn with_guide(mut value: Value) -> Value {
    let mut guides = Vec::new();
    let mut current = &value;
    loop {
        match current["encoding"].as_str() {
            Some("innen.edges.v1") => guides.push(EDGE_GUIDE),
            Some("innen.attachments.v1") => guides.push(ATTACHMENT_GUIDE),
            Some("innen.strings.v1" | "innen.rows.v1") => {
                guides.push(COMMON_GUIDE);
                break;
            }
            _ => break,
        }
        current = &current["page"];
    }
    if !guides.is_empty() {
        value["guide"] = json!(guides.join("\n"));
    }
    value
}

pub fn encode(page: &Page) -> Value {
    let original = serde_json::to_value(page).expect("page serializes");
    let mut groups: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
    for (index, record) in page.records.iter().enumerate() {
        let Some(event) = record.event.as_object() else {
            return original;
        };
        groups
            .entry(event.keys().cloned().collect())
            .or_default()
            .push(index);
    }
    let mut layouts = Vec::new();
    let mut rows = vec![Value::Null; page.records.len()];
    for (keys, indices) in groups {
        let first = &page.records[indices[0]].event;
        let mut shared = Map::new();
        let mut fields = Vec::new();
        for key in keys {
            if indices
                .iter()
                .all(|&i| page.records[i].event[&key] == first[&key])
            {
                shared.insert(key.clone(), first[&key].clone());
            } else {
                fields.push(key);
            }
        }
        for i in indices {
            let values: Vec<_> = fields
                .iter()
                .map(|key| page.records[i].event[key].clone())
                .collect();
            rows[i] = json!([page.records[i].line, layouts.len(), values]);
        }
        layouts.push(json!({"fields": fields, "shared": shared}));
    }
    let mut packed = original.clone();
    let object = packed.as_object_mut().expect("page object");
    object.remove("records");
    object.insert("encoding".into(), json!("innen.rows.v1"));
    object.insert("row_fields".into(), json!(["line", "layout", "values"]));
    object.insert("layouts".into(), json!(layouts));
    object.insert("rows".into(), json!(rows));
    let best = if packed.to_string().len() < original.to_string().len() {
        packed
    } else {
        original
    };
    factor_strings(best)
}

fn inventory<'a>(
    value: &'a Value,
    strings: &mut BTreeMap<&'a str, usize>,
    keys: &mut BTreeSet<&'a str>,
) {
    match value {
        Value::String(s) if s.len() >= 256 => {
            *strings.entry(s).or_default() += 1;
        }
        Value::Array(items) => {
            for item in items {
                inventory(item, strings, keys);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                keys.insert(key);
                inventory(item, strings, keys);
            }
        }
        _ => {}
    }
}

fn factor_strings(original: Value) -> Value {
    let mut counts = BTreeMap::new();
    let mut keys = BTreeSet::new();
    inventory(&original, &mut counts, &mut keys);
    let texts: Vec<_> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(s, _)| s.to_owned())
        .collect();
    if texts.is_empty() {
        return original;
    }
    // A literal source object must never be mistaken for an introduced reference.
    let mut ref_key = "$text".to_owned();
    while keys.contains(ref_key.as_str()) {
        ref_key.push('_');
    }
    let indices: BTreeMap<_, _> = texts
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    fn replace(value: &mut Value, indices: &BTreeMap<&str, usize>, key: &str) {
        match value {
            Value::String(s) => {
                if let Some(index) = indices.get(s.as_str()) {
                    *value = json!({key: index});
                }
            }
            Value::Array(items) => {
                for item in items {
                    replace(item, indices, key);
                }
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    replace(item, indices, key);
                }
            }
            _ => {}
        }
    }
    let mut page = original.clone();
    replace(&mut page, &indices, &ref_key);
    let packed =
        json!({"encoding":"innen.strings.v1", "ref_key":ref_key, "texts":texts, "page":page});
    if packed.to_string().len() < original.to_string().len() {
        packed
    } else {
        original
    }
}

/// Expand a compact page or return an ordinary page unchanged. Invalid compact
/// data fails explicitly rather than dropping rows or replacing missing values.
pub fn decode(value: &Value) -> Result<Value, String> {
    if value["encoding"] == "innen.edges.v1" {
        return decode(&super::deltas::decode(value)?);
    }
    if value["encoding"] == "innen.strings.v1" {
        let key = value["ref_key"].as_str().ok_or("missing reference key")?;
        let texts = value["texts"].as_array().ok_or("missing strings")?;
        if texts.iter().any(|s| !s.is_string()) {
            return Err("invalid string table".into());
        }
        let mut page = value
            .get("page")
            .filter(|p| p.is_object())
            .ok_or("missing page")?
            .clone();
        if page["encoding"] == "innen.strings.v1" {
            return Err("nested string encoding".into());
        }
        fn restore(value: &mut Value, texts: &[Value], key: &str) -> Result<(), String> {
            match value {
                Value::Object(object) if object.contains_key(key) => {
                    if object.len() != 1 {
                        return Err("ambiguous string reference".into());
                    }
                    let i = object[key]
                        .as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or("invalid string index")?;
                    *value = texts.get(i).ok_or("unknown string index")?.clone();
                }
                Value::Object(object) => {
                    for item in object.values_mut() {
                        restore(item, texts, key)?;
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        restore(item, texts, key)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        restore(&mut page, texts, key)?;
        return decode(&page);
    }
    if value.get("encoding").is_none() {
        return Ok(value.clone());
    }
    if value["encoding"] != "innen.rows.v1"
        || value["row_fields"] != json!(["line", "layout", "values"])
    {
        return Err("unsupported conversation encoding".into());
    }
    let layouts = value["layouts"].as_array().ok_or("missing layouts")?;
    let rows = value["rows"].as_array().ok_or("missing rows")?;
    let mut records = Vec::new();
    for row in rows {
        let row = row
            .as_array()
            .filter(|r| r.len() == 3)
            .ok_or("invalid row")?;
        row[0]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("invalid source line")?;
        let index = row[1]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("invalid layout index")?;
        let layout = layouts.get(index).ok_or("unknown layout")?;
        let fields = layout["fields"].as_array().ok_or("missing fields")?;
        let values = row[2]
            .as_array()
            .filter(|v| v.len() == fields.len())
            .ok_or("field count mismatch")?;
        let mut event = layout["shared"]
            .as_object()
            .ok_or("missing shared fields")?
            .clone();
        for (key, item) in fields.iter().zip(values) {
            let key = key.as_str().ok_or("invalid field name")?;
            if event.insert(key.into(), item.clone()).is_some() {
                return Err("duplicate field".into());
            }
        }
        records.push(json!({"line": row[0], "event": event}));
    }
    let mut expanded = value.clone();
    let object = expanded.as_object_mut().ok_or("invalid page")?;
    for key in ["encoding", "row_fields", "layouts", "rows", "guide"] {
        object.remove(key);
    }
    object.insert("records".into(), json!(records));
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{sources::Source, Record};

    fn page(events: Vec<Value>) -> Page {
        Page {
            session_id: "sample".into(),
            source: Source::Codex,
            source_path: "source.jsonl".into(),
            view: "events".into(),
            offset: 4,
            next_offset: Some(71),
            warnings: vec!["source truncated".into()],
            records: events
                .into_iter()
                .enumerate()
                .map(|(i, event)| Record {
                    line: i * 2 + 5,
                    event,
                })
                .collect(),
        }
    }

    #[test]
    fn repeated_nested_text_and_literal_reference_objects_roundtrip() {
        let text = "失敗是觀測，不可改寫為成功。\n".repeat(40);
        let p = page(
            (0..8)
                .map(|i| json!({"n":i,"payload":{"text":text,"other":i},"literal":{"$text":0}}))
                .collect(),
        );
        let compact = encode(&p);
        assert_eq!(compact["encoding"], "innen.strings.v1");
        assert_ne!(compact["ref_key"], "$text");
        assert_eq!(decode(&compact).unwrap(), serde_json::to_value(&p).unwrap());
        let guided = with_guide(compact.clone());
        assert!(guided["guide"].as_str().unwrap().contains("texts[index]"));
        assert_eq!(decode(&guided).unwrap(), serde_json::to_value(&p).unwrap());
        assert!(compact.to_string().len() < serde_json::to_string(&p).unwrap().len() / 2);
        let mut broken = compact;
        broken["texts"] = json!([]);
        assert!(decode(&broken).is_err());
    }

    #[test]
    fn roundtrip_preserves_order_exceptions_unknown_fields_and_metadata() {
        let p = page(
            (0..30)
                .map(|i| {
                    json!({"type":"tool_result", "timestamp": i,
            "content": if i == 7 {"FAIL: not approved"} else {"PASS is only a claim"},
            "unknown": {"nil":null,"array":[1,false,"\u{85}\n引用"]}})
                })
                .collect(),
        );
        let compact = encode(&p);
        assert_eq!(compact["encoding"], "innen.rows.v1");
        assert_eq!(
            decode(&with_guide(compact.clone())).unwrap(),
            serde_json::to_value(&p).unwrap()
        );
        assert_eq!(decode(&compact).unwrap(), serde_json::to_value(&p).unwrap());
        let mut broken = compact.clone();
        broken["rows"][0][2] = json!([]);
        assert!(decode(&broken).is_err());
        let mut broken = compact;
        broken["rows"][0][1] = json!(999);
        assert!(decode(&broken).is_err());
    }

    #[test]
    fn small_pages_fall_back_and_mixed_shapes_stay_in_source_order() {
        let empty = page(vec![]);
        assert_eq!(encode(&empty), serde_json::to_value(empty).unwrap());
        let mixed = page(
            (0..40)
                .map(|i| {
                    if i % 2 == 0 {
                        json!({"user":"same long repeated user input", "n":i})
                    } else {
                        json!({"assistant":"same long repeated assistant reply", "n":i})
                    }
                })
                .collect(),
        );
        assert_eq!(
            decode(&encode(&mixed)).unwrap(),
            serde_json::to_value(mixed).unwrap()
        );
    }
}

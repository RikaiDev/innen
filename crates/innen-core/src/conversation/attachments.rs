//! Opt-in source-backed references, not self-contained compression. No writes,
//! model calls, URI fetching or media interpretation. Hashes bind exact UTF-8
//! string values; the caller must retain the source to resolve a reference.
use super::{compact, sources::Source, Page, ReadError};
use crate::ids::sha256_hex;
use serde_json::{json, Value};

fn image_uri(text: &str) -> bool {
    let Some((header, body)) = text.split_once(',') else {
        return false;
    };
    header.starts_with("data:image/")
        && header.ends_with(";base64")
        && !body.is_empty()
        && body.len() % 4 == 0
        && body
            .trim_end_matches('=')
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        && body.len() - body.trim_end_matches('=').len() <= 2
}

fn walk(
    value: &mut Value,
    pointer: &str,
    encrypted: bool,
    record: usize,
    line: usize,
    refs: &mut Vec<Value>,
) {
    if let Some(text) = value.as_str() {
        let kind = if encrypted && pointer == "/payload/encrypted_content" && !text.is_empty() {
            Some("codex_encrypted_reasoning")
        } else if image_uri(text) {
            Some("image_data_uri")
        } else {
            None
        };
        if let Some(kind) = kind {
            refs.push(
                json!({"record":record,"line":line,"pointer":pointer,"kind":kind,
                "bytes":text.len(),"sha256":sha256_hex(text.as_bytes())}),
            );
            *value = json!({"attachment": refs.len()-1});
        }
        return;
    }
    match value {
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                walk(
                    item,
                    &format!("{pointer}/{i}"),
                    encrypted,
                    record,
                    line,
                    refs,
                );
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                let key = key.replace('~', "~0").replace('/', "~1");
                walk(
                    item,
                    &format!("{pointer}/{key}"),
                    encrypted,
                    record,
                    line,
                    refs,
                );
            }
        }
        _ => {}
    }
}

pub fn externalize(mut page: Page, use_compact: bool) -> Result<Value, ReadError> {
    if page.view != "events" {
        return Err(ReadError("--attachment-refs requires --view events".into()));
    }
    let original = if use_compact {
        compact::encode(&page)
    } else {
        serde_json::to_value(&page).expect("page serializes")
    };
    let mut refs = Vec::new();
    for (i, record) in page.records.iter_mut().enumerate() {
        let encrypted = page.source == Source::Codex
            && record.event["type"] == "response_item"
            && record.event["payload"]["type"] == "reasoning";
        walk(&mut record.event, "", encrypted, i, record.line, &mut refs);
    }
    if refs.is_empty() {
        return Ok(original);
    }
    let page = if use_compact {
        compact::encode(&page)
    } else {
        serde_json::to_value(page).expect("page serializes")
    };
    let packed = json!({"encoding":"innen.attachments.v1", "page":page, "attachments":refs,
        "notice":"Payloads are not inline. References need the unchanged source. No image or encrypted-content understanding is implied."});
    Ok(if packed.to_string().len() < original.to_string().len() {
        packed
    } else {
        original
    })
}

/// Resolve exactly one source event field, refusing changed values or a reader
/// cursor that skipped the requested physical row. Output is the original
/// string, including any data URI prefix; no decoding or execution occurs.
pub fn resolve(
    page: &Page,
    offset: usize,
    pointer: &str,
    expected: &str,
) -> Result<Value, ReadError> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ReadError("expected a 64-digit SHA-256".into()));
    }
    if !pointer.starts_with('/') || page.view != "events" || page.records.len() != 1 {
        return Err(ReadError(
            "attachment lookup requires one source event and an absolute JSON pointer".into(),
        ));
    }
    let record = &page.records[0];
    if offset.checked_add(1) != Some(record.line) {
        return Err(ReadError(
            "requested source row is no longer present".into(),
        ));
    }
    let text = record
        .event
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| ReadError("attachment string not found at source pointer".into()))?;
    let digest = sha256_hex(text.as_bytes());
    if !digest.eq_ignore_ascii_case(expected) {
        return Err(ReadError(
            "attachment hash mismatch: source changed or reference is incorrect".into(),
        ));
    }
    Ok(
        json!({"line":record.line,"pointer":pointer,"sha256":digest,"bytes":text.len(),"value":text}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Record;
    fn page(event: Value) -> Page {
        Page {
            session_id: "test".into(),
            source: Source::Codex,
            source_path: "source.jsonl".into(),
            view: "events".into(),
            offset: 0,
            next_offset: None,
            records: vec![Record { line: 1, event }],
            warnings: vec![],
        }
    }

    #[test]
    fn references_restore_exact_values_without_touching_literal_markers_or_text() {
        let image = format!("data:image/png;base64,{}", "YWJj".repeat(800));
        let p = page(
            json!({"type":"response_item","payload":{"type":"reasoning", "encrypted_content":"g".repeat(3000)},
            "a/b~c":[image],"literal":{"attachment":0},"text":"Do not publish; previous attempt failed."}),
        );
        let original = serde_json::to_value(&p).unwrap();
        let packed = externalize(p, true).unwrap();
        assert_eq!(packed["encoding"], "innen.attachments.v1");
        let mut restored = compact::decode(&packed["page"]).unwrap();
        for (index, reference) in packed["attachments"].as_array().unwrap().iter().enumerate() {
            let pointer = reference["pointer"].as_str().unwrap();
            let expected = original["records"][0]["event"].pointer(pointer).unwrap();
            assert_eq!(
                reference["sha256"],
                sha256_hex(expected.as_str().unwrap().as_bytes())
            );
            let slot = restored["records"][0]["event"]
                .pointer_mut(pointer)
                .unwrap();
            assert_eq!(*slot, json!({"attachment":index}));
            *slot = expected.clone();
        }
        assert_eq!(restored, original);
    }

    #[test]
    fn retrieval_fails_closed_on_changed_source_missing_pointer_and_wrong_row() {
        let p = page(json!({"image":"data:image/png;base64,YWJj"}));
        let hash = sha256_hex(b"data:image/png;base64,YWJj");
        assert_eq!(
            resolve(&p, 0, "/image", &hash).unwrap()["value"],
            p.records[0].event["image"]
        );
        assert!(resolve(&p, 0, "/image", &"0".repeat(64)).is_err());
        assert!(resolve(&p, 0, "/missing", &hash).is_err());
        assert!(resolve(&p, 1, "/image", &hash).is_err());
        assert!(resolve(&p, 0, "/image", "bad").is_err());
    }

    #[test]
    fn ordinary_text_unknown_encryption_and_invalid_data_uris_stay_inline() {
        let p = page(
            json!({"encrypted_content":"x".repeat(2000),"text":"example data:image/png;base64,YWJj",
            "image":"data:image/png;base64,not base64!"}),
        );
        let expected = serde_json::to_value(&p).unwrap();
        assert_eq!(externalize(p, false).unwrap(), expected);
    }
}

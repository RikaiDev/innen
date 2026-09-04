//! Canonical JSON + convergent event IDs (Task 2).

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: &str = "innen/v1";
pub const VOLATILE_NODE_EDGE: &[&str] = &["observed_utc"];

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(obj) => {
            let mut sorted = Map::with_capacity(obj.len());
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), canonical_value(&obj[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        other => other.clone(),
    }
}

/// Fixed-order envelope: serde emits struct fields in declaration order,
/// so the wire order is always v, op, payload regardless of the
/// `serde_json` map backend (default `BTreeMap` would otherwise re-sort
/// these three keys to op, payload, v). Payload objects are still
/// BTreeMap-sorted at every depth by [`canonical_value`].
#[derive(Serialize)]
struct Envelope<'a> {
    v: &'a str,
    op: &'a str,
    payload: Value,
}

pub fn canonical_bytes(op: &str, payload: &Value, volatile: &[&str]) -> Vec<u8> {
    let mut clean = canonical_value(payload);
    if let Value::Object(obj) = &mut clean {
        for k in volatile {
            obj.remove(*k);
        }
    }
    let env = Envelope {
        v: SCHEMA_VERSION,
        op,
        payload: clean,
    };
    serde_json::to_vec(&env).expect("canonical JSON serializes")
}

pub fn event_id(op: &str, payload: &Value) -> String {
    sha256_hex(&canonical_bytes(op, payload, VOLATILE_NODE_EDGE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn id_excludes_volatile_fields() {
        let a = event_id("node.upsert", &json!({"id": "t:1", "observed_utc": "2026-01-01T00:00:00Z"}));
        let b = event_id("node.upsert", &json!({"id": "t:1", "observed_utc": "2026-06-06T06:06:06Z"}));
        assert_eq!(a, b);
    }

    #[test]
    fn nested_keys_sorted_and_cjk_raw() {
        let bytes = canonical_bytes("node.upsert", &json!({"b": 1, "a": {"z": 1, "y": "臺"}}), VOLATILE_NODE_EDGE);
        assert_eq!(bytes, r#"{"v":"innen/v1","op":"node.upsert","payload":{"a":{"y":"臺","z":1},"b":1}}"#.as_bytes());
        let f = canonical_bytes("x", &json!({"n": 0.1}), &[]);
        assert!(f.windows(3).any(|w| w == b"0.1"));
    }
}

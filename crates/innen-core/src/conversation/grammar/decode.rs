//! Packet decoding: expand the dictionary in order, restore join markers, then
//! hand the page back to the compact-layer decoder and verify its source hash.

use super::{MAX_ATOMS, MAX_BYTES};
use crate::ids::sha256_hex;
use serde_json::{json, Value};

pub fn decode(packet: &Value) -> Result<Value, String> {
    if packet["encoding"] != super::ENCODING {
        return super::super::compact::decode(packet);
    }
    let key = packet["join_key"].as_str().ok_or("missing join key")?;
    let definitions = packet["dictionary"]
        .as_array()
        .ok_or("missing dictionary")?;
    if definitions.len() > MAX_ATOMS + super::MAX_RULES {
        return Err("dictionary exceeds bound".into());
    }
    fn join(parts: &[Value], prior: &[String]) -> Result<String, String> {
        let mut text = String::new();
        for part in parts {
            let fragment = if let Some(s) = part.as_str() {
                s
            } else {
                let i = part
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or("invalid grammar reference")?;
                prior.get(i).ok_or("forward, missing or cyclic reference")?
            };
            if text.len().saturating_add(fragment.len()) > MAX_BYTES {
                return Err("expanded string exceeds bound".into());
            }
            text.push_str(fragment);
        }
        Ok(text)
    }
    let mut prior = Vec::<String>::new();
    let mut expansion = 0usize;
    for definition in definitions {
        let text = if let Some(s) = definition.as_str() {
            s.to_owned()
        } else {
            join(
                definition.as_array().ok_or("invalid grammar definition")?,
                &prior,
            )?
        };
        expansion = expansion.saturating_add(text.len());
        if expansion > MAX_BYTES * 8 {
            return Err("dictionary expansion exceeds bound".into());
        }
        prior.push(text);
    }
    let mut page = packet.get("page").ok_or("missing page")?.clone();
    fn restore(
        value: &mut Value,
        key: &str,
        prior: &[String],
        budget: &mut usize,
    ) -> Result<(), String> {
        match value {
            Value::Object(o) if o.contains_key(key) => {
                if o.len() != 1 {
                    return Err("ambiguous grammar marker".into());
                }
                let text = join(o[key].as_array().ok_or("invalid grammar join")?, prior)?;
                *budget = budget.saturating_add(text.len());
                if *budget > MAX_BYTES * 8 {
                    return Err("page expansion exceeds bound".into());
                }
                *value = json!(text);
            }
            Value::Object(o) => {
                for x in o.values_mut() {
                    restore(x, key, prior, budget)?;
                }
            }
            Value::Array(a) => {
                for x in a {
                    restore(x, key, prior, budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    restore(&mut page, key, &prior, &mut 0)?;
    let original = super::super::compact::decode(&page)?;
    if packet["source_sha256"] != sha256_hex(original.to_string().as_bytes()) {
        return Err("conversation source hash mismatch".into());
    }
    Ok(original)
}

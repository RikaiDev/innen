//! Reading the document a located session points at.

use serde_json::Value;
use std::fs;

use super::{err, Located, ReadError, Source};

pub fn load_document(located: &Located) -> Result<Vec<Value>, ReadError> {
    let text = fs::read_to_string(&located.path).map_err(|e| err(&located.path, e))?;
    let data: Value = serde_json::from_str(&text).map_err(|e| err(&located.path, e))?;
    match located.source {
        Source::Gemini => data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| ReadError("Gemini document missing messages array".into())),
        Source::Vscode => {
            let requests = data
                .get("requests")
                .and_then(Value::as_array)
                .ok_or_else(|| ReadError("VS Code document missing requests array".into()))?;
            Ok(requests.clone())
        }
        _ => Err(ReadError("unsupported document source".into())),
    }
}

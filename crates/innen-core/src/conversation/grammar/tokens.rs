//! Reference-tokenizer accounting: the exact cost of what a caller will send,
//! measured with the reference BPE rather than with byte counts.

use serde_json::Value;
use std::sync::OnceLock;

fn tokenizer() -> Result<&'static tiktoken_rs::CoreBPE, String> {
    static BPE: OnceLock<Result<tiktoken_rs::CoreBPE, String>> = OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::o200k_base().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(Clone::clone)
}

pub fn tokens(value: &Value) -> Result<usize, String> {
    tokens_text(&value.to_string())
}

/// Count the exact JSON text a caller will send, using the reference tokenizer.
pub fn tokens_text(text: &str) -> Result<usize, String> {
    Ok(tokenizer()?.encode_ordinary(text).len())
}

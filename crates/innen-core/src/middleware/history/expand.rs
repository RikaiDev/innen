//! Return exact immutable decision envelopes for a still-matching history hash.

use super::model::{source_sha256, validate, Error, Request};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Return exact immutable decision envelopes only when the whole history input
/// still matches `expected_source_sha256`.
pub fn expand(
    request: Request,
    expected_source_sha256: &str,
    ids: &[String],
) -> Result<Value, Error> {
    validate(&request)?;
    let actual = source_sha256(&request);
    if expected_source_sha256 != actual {
        return Err(Error::SourceMismatch {
            expected: expected_source_sha256.to_owned(),
            actual,
        });
    }
    let known = request
        .events
        .iter()
        .map(|decision| decision.id.as_str())
        .collect::<BTreeSet<_>>();
    let unknown = ids
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(Error::UnknownDecisions { ids: unknown });
    }
    let wanted = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let events = request
        .events
        .into_iter()
        .filter(|decision| wanted.contains(decision.id.as_str()))
        .collect::<Vec<_>>();
    Ok(json!({
        "resolution": "expanded",
        "history_source_sha256": actual,
        "events": events,
    }))
}

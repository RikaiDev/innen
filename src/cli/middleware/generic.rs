pub(super) fn print_middleware_unknown(
    error: innen_core::middleware::Error,
    suppress_stdout: bool,
) -> i32 {
    let output = match error {
        innen_core::middleware::Error::RequiredDoesNotFit {
            budget_tokens,
            required_reference_tokens,
            required_ids,
        } => serde_json::json!({
            "resolution":"unknown",
            "reason":"required_context_exceeds_budget",
            "budget_tokens":budget_tokens,
            "required_reference_tokens":required_reference_tokens,
            "required_ids":required_ids,
        }),
        innen_core::middleware::Error::SourceMismatch { expected, actual } => serde_json::json!({
            "resolution":"unknown",
            "reason":"source_hash_mismatch",
            "expected_source_sha256":expected,
            "actual_source_sha256":actual,
        }),
        innen_core::middleware::Error::UnknownItems { ids } => serde_json::json!({
            "resolution":"unknown",
            "reason":"unknown_item_ids",
            "ids":ids,
        }),
        error => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    if suppress_stdout {
        eprintln!("{output}");
    } else {
        println!("{output}");
    }
    2
}

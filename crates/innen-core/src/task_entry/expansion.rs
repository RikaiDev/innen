//! The exact command that expands a brief into full node payloads.

use std::path::Path;

use super::types::TaskEntryOptions;

/// Structured argv for evidence expansion: same request, evidence view.
pub(super) fn evidence_argv(root: &Path, options: &TaskEntryOptions) -> Vec<String> {
    let mut argv = vec![
        "innen".to_string(),
        "query".to_string(),
        "--root".to_string(),
        root.to_string_lossy().into_owned(),
        "--q".to_string(),
        options.q.clone(),
        "--view".to_string(),
        "evidence".to_string(),
    ];
    if let Some(as_of_val) = &options.as_of {
        argv.push("--as-of".to_string());
        argv.push(as_of_val.clone());
    }
    if options.limit != 20 {
        argv.push("--limit".to_string());
        argv.push(options.limit.to_string());
    }
    if options.offset != 0 {
        argv.push("--offset".to_string());
        argv.push(options.offset.to_string());
    }
    if options.include_expired {
        argv.push("--include-expired".to_string());
    }
    argv
}

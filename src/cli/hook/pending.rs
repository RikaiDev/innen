//! List unprocessed pending snapshots, for SessionStart context.

use std::path::Path;

// ---------------------------------------------------------------------------
// pending
// ---------------------------------------------------------------------------

pub(super) fn pending_files(root: &Path) -> Vec<String> {
    let inbox = root.join("00-inbox/harvest");
    let mut names: Vec<String> = std::fs::read_dir(&inbox)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("pending-") && n.ends_with(".md"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

pub(super) fn cmd_hook_pending(root: &Path, format: &str) -> i32 {
    let names = pending_files(root);
    if crate::cli::util::is_human(format) {
        println!("pending\t{}", names.len());
        for n in &names {
            println!("file\t{n}");
        }
    } else {
        let list = names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"pending\":[{list}]}}");
    }
    0
}

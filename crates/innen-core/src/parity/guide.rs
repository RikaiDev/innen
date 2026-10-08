//! Short navigation text.

pub fn guide_text() -> &'static str {
    "innen is a local-first knowledge graph over an append-only journal.\n\
     \n\
     By intent:\n\
     \x20 find something recorded before   query <any words>\n\
     \x20 continue an unfinished session   unfinished, then pickup\n\
     \x20 read one known conversation      conversation <session-id>\n\
     \x20 see what a project still owes    project [id]\n\
     \x20 trace a clue to its source       trace <any words>\n\
     \x20 check whether state is sound     doctor\n\
     \n\
     query returns a compact task-context brief; project <id> --view evidence expands it, and --view full is the legacy page. trace needs wiki sync after wiki edits and returns expand_argv for exact records.\n\
     \n\
     Two outputs are not failures: harvest --check reports cursor=legacy_count_rebuilt when it re-lists an inbox whose cursor it cannot trust, and wiki sync exits 1 when some pages were skipped and left unchanged.\n\
     \n\
     Task status is recorded evidence; unknown status and heuristic conversation candidates are not confirmed unfinished work."
}

/// The active knowledge base root, or why none resolved. The guide is the
/// operational source of truth, so it has to say which root it is talking
/// about — a command that silently reads the wrong KB is worse than one that
/// refuses.
pub fn root_line(root: Option<&std::path::Path>, error: Option<&str>) -> String {
    match (root, error) {
        (Some(path), _) => format!("\nactive KB root: {}", path.display()),
        (None, Some(reason)) => {
            format!("\nactive KB root: unresolved ({reason}); pass --root <dir>")
        }
        (None, None) => "\nactive KB root: unresolved".to_string(),
    }
}

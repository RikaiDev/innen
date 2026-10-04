//! Short navigation text.

pub fn guide_text() -> &'static str {
    "innen is a local-first knowledge graph over an append-only journal.\n\
     What remains? project [id] returns compact recorded tasks across projects or one project.\n\
     Need evidence? project <id> --view evidence; legacy project page: --view full.\n\
     Find prior knowledge: query --q <term>. Continue a conversation: resume <session-id>.\n\
     Trace original sources: wiki sync after wiki edits, then trace --q <clue>; use returned expand_argv for exact records.\n\
     Task status is recorded evidence; unknown status and heuristic conversation candidates are not confirmed unfinished work."
}

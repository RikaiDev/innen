//! Task-context entry: evidence-backed resolution of intent, assets, projects,
//! and baselines without requiring project IDs or falling back to cwd.

mod admission;
mod entry;
mod node_identity;
mod query_terms;
mod render;
mod traversal;
mod types;

pub use entry::task_entry;
pub use node_identity::node_searchable_text;
pub use query_terms::{extract_query_terms, has_asset_edit_intent};
pub use render::render;
pub use types::{CandidateItem, TaskEntryOptions};

#[cfg(test)]
mod tests;

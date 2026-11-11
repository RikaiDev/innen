//! Projection of canonical wiki Markdown into the append-only graph.
mod identity;
mod parse;
mod projection;
mod sync;
mod types;
mod verify;

pub use sync::sync;
pub use types::{SyncReport, WikiGraphError, MANAGED_BY};
pub use verify::{VerifyReport, verify};

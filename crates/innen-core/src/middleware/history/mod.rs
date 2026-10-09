//! Evidence-bearing, task-scoped decision-history selection.
//!
//! The caller owns immutable chronological decision events.  This module only
//! validates their graph and projects the current task's effective decisions
//! into the bounded generic middleware packet.  It never turns a guarded or
//! superseded statement into an active instruction.

mod ancestry;
mod expand;
mod model;
mod prepare;

pub mod store;

pub use expand::expand;
pub(crate) use model::validate;
pub use model::{
    source_sha256, Decision, Error, PreparedHistory, Request, Source, Task, TaskState,
};
pub use prepare::prepare;

#[cfg(test)]
mod tests;

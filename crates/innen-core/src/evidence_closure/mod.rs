//! Contract-scoped evidence closure over the canonical journal, not a new index.
//! Source grants and fact alternatives are caller assertions, not authenticated
//! permissions or machine-proved entailment. Every selected dependency is pinned.
mod closure;
mod contract;
mod evaluation;
mod snapshot;

pub use contract::{proposal_context, Contract, Requirement, SourceGrant};
pub use evaluation::{evaluate, query, read_journal};

#[cfg(test)]
mod tests;

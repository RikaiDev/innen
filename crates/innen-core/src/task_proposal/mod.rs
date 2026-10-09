//! External-model task proposals remain untrusted until source and controller review.
//! Citation matching is necessary, not proof of entailment or completeness.
mod evaluate;
mod id_context;
mod proposal;
mod review;
#[cfg(test)]
mod tests;
pub use evaluate::evaluate;
pub use id_context::{bind_draft, id_context, DraftEnvelope, IdDraft, IdFact, IdStatement};
pub use proposal::{Citation, Disposition, Proposal, ProposedFact, Statement};
pub use review::{Review, ReviewPremise, ReviewSpan};

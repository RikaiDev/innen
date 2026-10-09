//! Discovery of recent unfinished conversations based on structural evidence.
//!
//! A session is classified as a `candidate`, never as a semantic certainty.
//! Structural indicators include active/blocked checkpoints, trailing unanswered
//! user requests, interrupted/cancelled turns, and unresolved tool failures.
//! Sessions with explicit `completed` or `superseded` checkpoints are excluded.

mod discover;
mod evidence;
mod model;
mod since;

pub use discover::{find_all_unfinished_candidates, find_unfinished_candidates};
pub use model::{Confidence, Reason, UnfinishedCandidate};
pub use since::{parse_since, rfc3339_to_secs};

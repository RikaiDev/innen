//! Bounded, provenance-aware retrieval.
mod cache;
mod expand;
mod graph;
mod lexical;
mod metadata;
mod search;
mod source;
mod types;

pub use expand::expand;
pub use search::search;
pub use types::{Error, Options};

#[cfg(test)]
mod tests;

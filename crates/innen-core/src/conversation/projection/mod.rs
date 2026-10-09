//! Text projection only; source-native events remain available in events view.

mod context;
mod dialogue;
mod index;

#[cfg(test)]
mod tests;

pub use context::context;
pub use dialogue::dialogue;
pub use index::{index, link_index};

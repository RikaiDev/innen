//! Parity surfaces, one job per file.
//!
//! Each module owns a single render or report: navigation text, journal
//! counts, journal order, the project page, and the profile page.

mod guide;
pub(crate) mod profile;
mod project_render;
mod status;
mod timeline;

pub use guide::{guide_text, root_line};
pub use profile::profile_render;
pub use project_render::project_render;
pub use status::{status, StatusReport};
pub use timeline::{timeline, TimelineEntry};

#[cfg(test)]
mod tests;

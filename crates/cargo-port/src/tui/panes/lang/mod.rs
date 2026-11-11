mod constants;
#[cfg(test)]
mod readability_tests;
mod render;

mod pane;
pub use pane::LangPane;
pub(super) use render::minimum_width;
pub(in crate::tui) use render::selected_language_stats;
pub(super) use render::*;

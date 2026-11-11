mod constants;
mod pane;
#[cfg(test)]
mod readability_tests;
mod render;
#[cfg(test)]
mod value_fit_tests;

#[cfg(test)]
mod sync_column_tests;

pub use pane::GitPane;
pub(super) use render::GitDescription;
pub(super) use render::git_content_height;
pub(super) use render::git_lower_content_height;
use render::render_git_pane_body;

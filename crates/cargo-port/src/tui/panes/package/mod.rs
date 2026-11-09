#[cfg(test)]
mod ci_count_tests;
mod constants;
#[cfg(test)]
mod disk_loading_tests;
mod pane;
#[cfg(test)]
mod readability_tests;
mod render;

pub use pane::PackagePane;
pub use render::detail_column_scroll_offset;
pub(super) use render::manifest_error_lines;
pub(super) use render::package_content_height;
pub(super) use render::package_lower_metadata_height;
use render::render_package_pane_body;

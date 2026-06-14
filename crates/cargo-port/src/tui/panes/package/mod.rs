mod pane;
mod render;

pub use pane::PackagePane;
pub use render::detail_column_scroll_offset;
pub(super) use render::package_content_height;
#[cfg(test)]
pub use render::package_label_width;
pub(super) use render::package_lower_metadata_height;
use render::render_package_pane_body;
#[cfg(test)]
pub use render::stats_column_width;

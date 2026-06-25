mod pane;
mod render;

pub use pane::PackagePane;
pub use render::detail_column_scroll_offset;
pub(super) use render::package_content_height;
pub(super) use render::package_lower_metadata_height;
use render::render_package_pane_body;

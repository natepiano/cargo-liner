mod pane;
mod render;

pub use pane::GitPane;
pub(super) use render::git_content_height;
pub(super) use render::git_lower_content_height;
use render::render_git_pane_body;

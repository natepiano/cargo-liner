mod data;
mod render;

pub use data::LintsData;
#[cfg(test)]
pub use data::LintsProjectKind;
pub use data::build_lints_data;
pub use render::render_lints_pane_body;

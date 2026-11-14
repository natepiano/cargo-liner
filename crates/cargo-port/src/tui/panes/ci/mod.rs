mod data;
#[cfg(test)]
mod readability_tests;
mod render;

pub use data::CiData;
#[cfg(test)]
pub use data::CiEmptyState;
pub use data::build_ci_data;
pub(in crate::tui) use render::CiTablePlan;
pub use render::render_ci_pane_body;

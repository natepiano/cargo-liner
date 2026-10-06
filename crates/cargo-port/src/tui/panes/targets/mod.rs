//! Targets pane render body.
//!
//! Each target occupies one row in the table, which fills the pane body.

mod constants;
mod data;
mod pane;
mod render;

pub use data::BuildMode;
pub use data::RunTargetKind;
pub use data::TargetEntry;
#[cfg(test)]
pub use data::TargetSource;
pub use data::TargetsData;
pub use data::build_target_list_from_data;
pub use data::lookup_targets_data;
pub use pane::TargetsPane;
use render::render_targets_pane_body;

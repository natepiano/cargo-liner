//! Targets pane: its data, its pane type and its render body.
//!
//! Each target occupies one row in the table, which fills the pane body.

#[cfg(test)]
mod availability_tests;
mod constants;
mod data;
mod pane;
#[cfg(test)]
mod readability_tests;

#[cfg(test)]
mod width_tests;
#[cfg(test)]
mod worktree_source_tests;

pub use data::BuildMode;
pub use data::RunTargetKind;
pub use data::TargetEntry;
#[cfg(test)]
pub use data::TargetSource;
pub use data::TargetsData;
pub use data::build_target_list_from_data;
pub use data::lookup_targets_data;
pub use pane::TargetAvailability;
pub use pane::TargetsContent;
pub use pane::TargetsPane;
pub(super) use pane::minimum_width;

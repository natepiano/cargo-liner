//! Targets pane render body.
//!
//! Entry: `TargetsPane::render` in `pane.rs` calls
//! `render_targets_pane_body`, which delegates to the data /
//! empty branches below. The pane is two boxes: the targets table
//! (one row per target, a `Fill` box) above the Running sub-pane
//! (every running instance across all tracked workspaces, a box
//! capped at `RUNNING_CAP_PERCENT` of the inner height that is
//! present only while anything runs).

mod constants;
mod data;
mod pane;
mod render;
mod running_subpane;

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
pub use running_subpane::CargoGroup;
pub use running_subpane::RunningListRow;
pub use running_subpane::build_running_list;
pub use running_subpane::build_running_rows;
pub use running_subpane::format_start_age;
pub use running_subpane::outline_subtree_len;
pub use running_subpane::resolve_kill_request;

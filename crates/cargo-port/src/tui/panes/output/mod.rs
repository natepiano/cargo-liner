//! Output pane render body.
//!
//! Entry: `OutputPane::render` in `pane.rs` calls
//! `render_output_pane_body`. The body reads `OwnedRun` output from
//! `PaneRenderCtx::inflight` and the pane's own cursor /
//! selection / follow state from `OutputPane`.
mod constants;
mod hit_map;
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod interaction_tests;
mod monitor_render;
mod pane;
mod presentation;
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod presentation_tests;
mod render;
mod selection;

pub use hit_map::OutputMonitorHit;
pub use pane::CapturedOutputRow;
pub use pane::OutputPane;
pub use pane::OutputTabStep;
pub use presentation::OutputCopyAvailability;
pub use presentation::OutputMonitorVisibility;
pub use presentation::OutputPaneVisibility;
pub use presentation::OutputPresentation;
use render::render_output_pane_body;
pub use selection::ColumnSelection;
/// Named outside the pane only where a test asserts the selected rows.
#[cfg(test)]
pub use selection::OutputSelectionRange;
pub use selection::OwnedColumnSelection;
pub use selection::VisualSelectionPermission;

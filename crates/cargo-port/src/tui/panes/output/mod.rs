//! Output pane render body.
//!
//! Entry: `OutputPane::render` in `pane.rs` calls
//! `render_output_pane_body`. The body reads `OwnedRun` output from
//! `PaneRenderCtx::inflight` and the pane's own selection / follow state
//! from `OutputPane`.
mod pane;
mod presentation;
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod presentation_tests;
mod render;
mod selection;

pub use pane::CapturedOutputRow;
pub use pane::OutputPane;
pub use presentation::OutputCopyAvailability;
pub use presentation::OutputPaneVisibility;
pub use presentation::OutputPresentation;
use render::render_output_pane_body;
/// Named outside the pane only where a test asserts the selected rows.
#[cfg(test)]
pub use selection::OutputSelectionRange;

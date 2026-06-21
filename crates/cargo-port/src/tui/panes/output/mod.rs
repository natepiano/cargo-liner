//! Output pane render body.
//!
//! Entry: `OutputPane::render` in `pane.rs` calls
//! `render_output_pane_body`. The body reads in-flight example
//! state from `PaneRenderCtx::inflight` and the pane's own cursor /
//! selection / follow state from `OutputPane`.
mod render;

mod pane;
mod selection;
pub use pane::OutputPane;
use render::render_output_pane_body;

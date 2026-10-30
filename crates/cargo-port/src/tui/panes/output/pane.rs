use std::ops::Deref;
use std::ops::DerefMut;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CopySelectionResult;
use tui_pane::Hittable;
use tui_pane::PaneFrameChrome;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::StreamingTextSelection;
use tui_pane::Viewport;

use super::selection::OutputSelection;
use super::selection::OutputSelectionRange;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::render_context::PaneRenderCtx;

pub struct OutputPane {
    pub focus: RenderFocus,
    selection: StreamingTextSelection,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            focus:     RenderFocus::inactive(),
            selection: StreamingTextSelection::new(),
        }
    }

    /// The current selection state.
    pub const fn selection(&self) -> &OutputSelection { self.selection.selection() }

    /// Mutable viewport access for const pane routing.
    pub(crate) const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.selection.viewport }

    /// The captured-output row under `pos`, or [`CapturedOutputRow::Outside`]
    /// when the position is off the pane's body.
    pub fn captured_output_row_at(&self, pos: Position) -> CapturedOutputRow {
        self.viewport
            .pos_to_local_row(pos)
            .map_or(CapturedOutputRow::Outside, CapturedOutputRow::Inside)
    }

    /// Build the clipboard payload for the current selection, reading the
    /// frozen snapshot when pinned or `live` while following the tail.
    pub fn copy_payload(&self, live: &[String]) -> CopySelectionResult {
        let OutputSelectionRange::Rows { first, last } = self.selected_range(live) else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_output(
            self.selection().visual_selection_source().lines(live),
            first,
            last,
        )
    }

    /// Resume following final output unless a visual range is selected.
    pub fn on_process_exit(&mut self) { self.selection.finish_stream(); }
}

impl Deref for OutputPane {
    type Target = StreamingTextSelection;
    fn deref(&self) -> &Self::Target { &self.selection }
}

impl DerefMut for OutputPane {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.selection }
}

/// What a screen position names in Cargo Port's own captured output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedOutputRow {
    /// The position is not over a captured-output row.
    Outside,
    /// The buffer index of the captured-output row under the position.
    Inside(usize),
}

impl Renderable<PaneRenderCtx<'_>> for OutputPane {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &PaneRenderCtx<'_>,
    ) -> Option<PaneFrameChrome> {
        Some(super::render_output_pane_body(frame, area, self, ctx))
    }
}

impl Hittable<HoverTarget> for OutputPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Output,
            row,
        })
    }
}

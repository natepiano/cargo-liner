use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::PaneFrameChrome;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::TargetsData;
use crate::tui::render_context::PaneRenderCtx;

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
    content:      Option<TargetsData>,
    /// Per-rendered-row hit-test geometry.
    row_rects:    Vec<(Rect, usize)>,
}

impl TargetsPane {
    pub const fn new() -> Self {
        Self {
            viewport:  Viewport::new(),
            focus:     RenderFocus::inactive(),
            content:   None,
            row_rects: Vec::new(),
        }
    }

    pub const fn content(&self) -> Option<&TargetsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: TargetsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Hittable<HoverTarget> for TargetsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Targets,
            row,
        })
    }
}

impl Renderable<PaneRenderCtx<'_>> for TargetsPane {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        _: &PaneRenderCtx<'_>,
    ) -> Option<PaneFrameChrome> {
        Some(super::render_targets_pane_body(frame, area, self))
    }
}

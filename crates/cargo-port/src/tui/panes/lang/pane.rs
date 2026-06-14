use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::lang;
use crate::tui::panes::package::RenderStyles;
use crate::tui::render_context::PaneRenderCtx;

// ── Lang ────────────────────────────────────────────────────────
pub struct LangPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
}

impl LangPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            focus:    RenderFocus::inactive(),
        }
    }
}

impl Renderable<PaneRenderCtx<'_>> for LangPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        lang::render_lang_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for LangPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Lang,
            row,
        })
    }
}

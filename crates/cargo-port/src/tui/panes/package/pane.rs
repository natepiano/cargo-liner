use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PackageData;
use crate::tui::panes::PaneId;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::package;
use crate::tui::render_context::PaneRenderCtx;

// ── Package ─────────────────────────────────────────────────────
pub struct PackagePane {
    pub viewport:        Viewport,
    pub focus:           RenderFocus,
    content:             Option<PackageData>,
    row_rects:           Vec<(Rect, usize)>,
    /// Scroll offset of the Tests box in the stats column, held across
    /// frames so the box stays put while the cursor is on a pinned row.
    /// Separate from `viewport.scroll_offset`, which the metadata column owns.
    tests_scroll_offset: usize,
}

impl PackagePane {
    pub const fn new() -> Self {
        Self {
            viewport:            Viewport::new(),
            focus:               RenderFocus::inactive(),
            content:             None,
            row_rects:           Vec::new(),
            tests_scroll_offset: 0,
        }
    }

    pub const fn content(&self) -> Option<&PackageData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: PackageData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }

    pub const fn tests_scroll_offset(&self) -> usize { self.tests_scroll_offset }

    pub const fn set_tests_scroll_offset(&mut self, offset: usize) {
        self.tests_scroll_offset = offset;
    }
}

impl Renderable<PaneRenderCtx<'_>> for PackagePane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        package::render_package_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for PackagePane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Package,
            row,
        })
    }
}

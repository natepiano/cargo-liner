use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::tui::pane::DismissTarget;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes::PaneId;
use crate::tui::panes::project_list;

// ── ProjectList ─────────────────────────────────────────────────
pub struct ProjectListPane {
    pub viewport:    Viewport,
    pub focus:       RenderFocus,
    /// Per-row dismiss `[x]` rects recorded each frame, alongside
    /// the resolved `DismissTarget`. The action region wins over
    /// the row body in `Hittable::hit_test_at`.
    dismiss_actions: Vec<(Rect, DismissTarget)>,
    /// Rect occupied by the list body, recorded during render and
    /// read by input dispatch for click / scroll hit-testing.
    pub body_rect:   Rect,
}

impl ProjectListPane {
    pub const fn new() -> Self {
        Self {
            viewport:        Viewport::new(),
            focus:           RenderFocus::inactive(),
            dismiss_actions: Vec::new(),
            body_rect:       Rect::ZERO,
        }
    }

    pub fn set_dismiss_actions(&mut self, actions: Vec<(Rect, DismissTarget)>) {
        self.dismiss_actions = actions;
    }
}

impl Renderable<PaneRenderCtx<'_>> for ProjectListPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        project_list::render_project_list_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for ProjectListPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        for (rect, target) in &self.dismiss_actions {
            if rect.contains(pos) {
                return Some(HoverTarget::Dismiss(target.clone()));
            }
        }
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::ProjectList,
            row,
        })
    }
}

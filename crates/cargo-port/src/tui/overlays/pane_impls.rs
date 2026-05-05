//! `Pane` and `Hittable` impls for the three overlay panes.
//!
//! Render is a no-op for all three — the popup body is drawn by
//! `keymap_ui::render_keymap_popup`, `settings::render_settings_popup`,
//! and `finder::render_finder_popup` directly from the top-level
//! `render::ui` path. These impls exist so each pane has a typed home
//! for `Hittable::hit_test_at`, which the click/hover dispatch in
//! `tui/interaction.rs::hit_test_at` walks via `HITTABLE_Z_ORDER`.

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::FinderPane;
use super::KeymapPane;
use super::SettingsPane;
use crate::tui::pane::Hittable;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes;
use crate::tui::panes::PaneId;

impl Pane for KeymapPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Hittable for KeymapPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Keymap,
            row,
        })
    }
}

impl Pane for SettingsPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Hittable for SettingsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let inner = self.viewport.content_area();
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        if !inner.contains(pos) {
            return None;
        }
        let line_index = usize::from(pos.y.saturating_sub(inner.y));
        let row = self.line_targets.get(line_index).copied().flatten()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Settings,
            row,
        })
    }
}

impl Pane for FinderPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Hittable for FinderPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = panes::hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Finder,
            row,
        })
    }
}

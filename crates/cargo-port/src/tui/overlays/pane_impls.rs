//! `Pane` impls for overlay panes plus `Hittable` for `FinderPane`.
//!
//! Render is a no-op for the framework-owned overlays — the popup
//! bodies are drawn by `keymap_ui::render_keymap_popup`,
//! `settings::render_settings_popup`, and `finder::render_finder_popup`
//! directly from the top-level `render::ui` path.
//!
//! Hit-testing for `KeymapPane` and `SettingsPane` lives in
//! `tui_pane` (their `row_at(pos)` methods); cargo-port consumes those
//! through `Framework::hit_test_at` via the
//! `InputContext::map_framework_hit` hook in `tui/interaction.rs`.
//! The finder is app-owned, so its `Hittable` impl stays here.

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::KeymapPane;
use tui_pane::SettingsPane;

use super::FinderPane;
use crate::tui::pane::Hittable;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes;
use crate::tui::panes::PaneId;

impl Pane for KeymapPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Pane for SettingsPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Pane for FinderPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {}
}

impl Hittable<HoverTarget> for FinderPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = panes::hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Finder,
            row,
        })
    }
}

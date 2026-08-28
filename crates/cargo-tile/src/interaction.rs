//! Where a click lands.
//!
//! The framework owns the order a click is offered around — toasts,
//! then any open framework overlay, then whatever the app tiles
//! underneath — so nothing here re-derives it. [`App`] supplies the two
//! app-side pieces that ladder asks for, [`tui_pane::dispatch_hit_test`]
//! walks it, and [`handle_click`] acts on what comes back.

use ratatui::layout::Position;
use tui_pane::FrameworkHit;
use tui_pane::FrameworkOverlayId;
use tui_pane::HitTestRegistry;
use tui_pane::Hittable;
use tui_pane::InputContext;
use tui_pane::ModalHit;
use tui_pane::Viewport;

use crate::app::App;
use crate::app::AppPaneId;
use crate::tiles::TileGrid;

/// Every pane a click can land on, top of the stack first. The tile
/// grid fills the whole body, so there is only the one.
const HIT_TEST_Z_ORDER: [AppPaneId; 1] = [AppPaneId::Main];

/// What a click found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Picked {
    /// A cell of the tile grid, by cell number.
    Cell(usize),
    /// A row of an open framework overlay.
    OverlayRow {
        /// The overlay the row belongs to.
        id:  FrameworkOverlayId,
        /// The row's index inside it.
        row: usize,
    },
}

/// Act on the click at `pos`.
///
/// Clicking a cell is how focus moves without the arrow keys, and it
/// reads the same way: the cell lights up and stays lit until something
/// takes it out of the grid.
pub(crate) fn handle_click(app: &mut App, pos: Position) {
    match tui_pane::dispatch_hit_test(app, pos) {
        Some(Picked::Cell(index)) => app.tiles.focus_cell(index),
        Some(Picked::OverlayRow { id, row }) => overlay_row(app, id, row),
        None => (),
    }
}

/// Move an overlay's selection to the row that was clicked.
const fn overlay_row(app: &mut App, id: FrameworkOverlayId, row: usize) {
    let viewport = match id {
        FrameworkOverlayId::Settings => app.framework.settings_pane.viewport_mut(),
        FrameworkOverlayId::Keymap => app.framework.keymap_pane.viewport_mut(),
        FrameworkOverlayId::GlobalShortcuts => app.framework.global_shortcuts_pane.viewport_mut(),
    };
    viewport.set_pos(row);
}

impl Hittable<Picked> for TileGrid {
    fn hit_test_at(&self, pos: Position) -> Option<Picked> { self.cell_at(pos).map(Picked::Cell) }
}

impl HitTestRegistry for App {
    type PaneId = AppPaneId;
    type Target = Picked;

    fn z_order() -> &'static [AppPaneId] { &HIT_TEST_Z_ORDER }

    fn pane(&self, id: AppPaneId) -> Option<&dyn Hittable<Picked>> {
        match id {
            AppPaneId::Main => Some(&self.tiles),
            // The attract screen has no click behavior, and the
            // favorites modal absorbs clicks before this pane walk.
            AppPaneId::Attract(_) | AppPaneId::Favorites => None,
        }
    }

    /// The grid tracks focus by cell rather than through a viewport, so
    /// there is nothing here for the framework's hover pass to clear.
    fn viewport_mut(&mut self, _: AppPaneId) -> Option<&mut Viewport> { None }
}

impl InputContext for App {
    fn framework_hit(&self, pos: Position) -> Option<FrameworkHit> {
        self.framework.hit_test_at(pos)
    }

    /// The favorites modal has no mouse selection yet, but still
    /// absorbs every click so none reaches the grid underneath it.
    fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<Picked> {
        if self.favorites_overlay.is_open() {
            ModalHit::MissedRow
        } else {
            ModalHit::Closed
        }
    }

    /// Toasts and overlay chrome are framework surfaces. An overlay row
    /// is the only framework hit that becomes an app action; every
    /// other hit has already been absorbed.
    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<Picked> {
        match hit {
            FrameworkHit::Overlay { id, row } => Some(Picked::OverlayRow { id, row }),
            FrameworkHit::Toast(_) | FrameworkHit::ModalMissed => None,
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn open_app_modal_absorbs_clicks_before_the_grid() {
        let mut app = App::new_for_test().expect("test app should build");
        assert_eq!(
            app.app_modal_overlay_hit(Position::new(0, 0)),
            ModalHit::Closed
        );

        let current_parameters = app.attract.current_settings().into();
        let keymap = Rc::clone(&app.keymap);
        app.favorites_overlay.open(&keymap, current_parameters);
        assert_eq!(
            app.app_modal_overlay_hit(Position::new(0, 0)),
            ModalHit::MissedRow
        );
    }
}

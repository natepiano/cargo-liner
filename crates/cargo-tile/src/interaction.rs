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
            // The attract screen is drawn over everything and answers
            // no click: its pane holds a keymap scope and nothing else.
            AppPaneId::Attract(_) => None,
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

    /// This app owns no modal of its own; every overlay it opens is one
    /// the framework runs.
    fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<Picked> { ModalHit::Closed }

    /// Toasts are the framework's, and this app raises none, so an
    /// overlay row is the only framework hit worth acting on. Anything
    /// else is a click the framework has already absorbed.
    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<Picked> {
        match hit {
            FrameworkHit::Overlay { id, row } => Some(Picked::OverlayRow { id, row }),
            FrameworkHit::Toast(_) | FrameworkHit::ModalMissed => None,
        }
    }
}

//! Where a click lands.
//!
//! The framework owns the order a click is offered around — toasts,
//! then any open framework overlay, then whatever the app tiles
//! underneath — so nothing here re-derives it. [`App`] supplies the two
//! app-side pieces that ladder asks for, and
//! [`tui_pane::handle_tile_click`] walks it and acts on the
//! [`TilePick`] that comes back.

use ratatui::layout::Position;
use tui_pane::FrameworkHit;
use tui_pane::HitTestRegistry;
use tui_pane::Hittable;
use tui_pane::InputContext;
use tui_pane::ModalHit;
use tui_pane::TilePick;
use tui_pane::Viewport;

use crate::app::App;
use crate::app::AppPaneId;

/// Every pane a click can land on, top of the stack first. The tile
/// grid fills the whole body, so there is only the one.
const HIT_TEST_Z_ORDER: [AppPaneId; 1] = [AppPaneId::Main];

impl HitTestRegistry for App {
    type PaneId = AppPaneId;
    type Target = TilePick;

    fn z_order() -> &'static [AppPaneId] { &HIT_TEST_Z_ORDER }

    fn pane(&self, id: AppPaneId) -> Option<&dyn Hittable<TilePick>> {
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
    fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<TilePick> {
        if self.favorites_overlay.is_open() {
            ModalHit::MissedRow
        } else {
            ModalHit::Closed
        }
    }

    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<TilePick> {
        TilePick::from_framework_hit(hit)
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tui_pane::FavoritesFileState;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::GlobalAction;
    use tui_pane::TABLE_CELL;
    use tui_pane::TerminalApp;
    use tui_pane::TileAction;

    use super::*;
    use crate::constants::STATUS_LINE_HEIGHT;
    use crate::render;
    use crate::tiles::TileContent;

    /// Width of the frame the click tests draw.
    const WIDTH: u16 = 80;
    /// Height of the frame the click tests draw.
    const HEIGHT: u16 = 24;

    /// The body the grid is drawn in: every row but the status line.
    const fn body() -> Rect { Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT) }

    /// Draw one frame of `app`, which is what records where the grid's
    /// cells and an open overlay's rows sit for a click to find.
    fn draw_frame(app: &mut App) {
        let keymap = Rc::clone(&app.keymap);
        let mut terminal =
            Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("the test terminal opens");
        terminal
            .draw(|frame| render::draw(frame, app, &keymap))
            .expect("the test terminal draws");
    }

    /// An app whose grid holds the summary and one empty cell, settled
    /// and drawn.
    fn two_cell_app() -> App {
        let mut app = App::new_for_test().expect("test app should build");
        draw_frame(&mut app);
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        app.tiles.apply(TileAction::Add, initial_rows);
        app.tiles.settle_for_test();
        draw_frame(&mut app);
        app
    }

    /// What the cell holding the focus ring shows.
    fn focused(app: &App) -> TileContent {
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        app.tiles
            .placements(body(), initial_rows)
            .into_iter()
            .find(|placement| placement.frame.is_focused())
            .map(|placement| placement.content)
            .expect("one cell holds the focus ring")
    }

    /// The middle of the cell showing `content`.
    fn middle_of(app: &App, content: &TileContent) -> Position {
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        let rect = app
            .tiles
            .placements(body(), initial_rows)
            .into_iter()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.rect())
            .expect("the grid draws the cell");
        Position::new(rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    /// A click on the second cell of a two-cell grid moves the ring
    /// onto it.
    #[test]
    fn a_click_on_a_cell_focuses_it() {
        let mut app = two_cell_app();
        assert_eq!(focused(&app), TileContent::Summary);

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        app.click(cell);

        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
    }

    /// A click on the summary takes the ring back from the cell after
    /// it.
    #[test]
    fn a_click_on_the_summary_focuses_it() {
        let mut app = two_cell_app();
        app.tiles.focus_cell(TABLE_CELL + 1);
        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));

        let summary = middle_of(&app, &TileContent::Summary);
        app.click(summary);

        assert_eq!(focused(&app), TileContent::Summary);
    }

    /// The open favorites overlay takes a click on the cell under it,
    /// so the ring stays where it was.
    #[test]
    fn a_click_under_the_favorites_overlay_leaves_the_ring_in_place() {
        let mut app = two_cell_app();
        tui_pane::open_favorites_on_state_for_test(
            &mut app,
            FavoritesFileState::Missing {
                path: PathBuf::from("/tmp/favorites.toml"),
            },
        );

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        app.click(cell);

        assert_eq!(focused(&app), TileContent::Summary);
    }

    /// The row the open framework overlay has selected, or `None` when
    /// no framework overlay is open.
    fn selected(app: &App) -> Option<usize> {
        let viewport = match app.framework.overlay()? {
            FrameworkOverlayId::Settings => app.framework.settings_pane.viewport(),
            FrameworkOverlayId::Keymap => app.framework.keymap_pane.viewport(),
            FrameworkOverlayId::GlobalShortcuts => app.framework.global_shortcuts_pane.viewport(),
        };
        Some(viewport.pos())
    }

    /// A click on a row of each framework overlay selects that row.
    #[test]
    fn a_click_on_a_framework_overlay_row_selects_it() {
        for opener in [
            GlobalAction::OpenSettings,
            GlobalAction::OpenKeymap,
            GlobalAction::OpenGlobalShortcuts,
        ] {
            let mut app = App::new_for_test().expect("test app should build");
            let keymap = Rc::clone(&app.keymap);
            keymap.dispatch_framework_global(opener, &mut app);
            draw_frame(&mut app);
            let before = selected(&app).expect("the overlay opened");
            let (position, row) = (0..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| Position::new(x, y)))
                .find_map(|position| match app.framework.hit_test_at(position) {
                    Some(FrameworkHit::Overlay { row, .. }) if row != before => {
                        Some((position, row))
                    },
                    _ => None,
                })
                .expect("the overlay draws a second row to click");

            app.click(position);

            assert_eq!(selected(&app), Some(row), "{opener:?}");
        }
    }

    #[test]
    fn open_app_modal_absorbs_clicks_before_the_grid() {
        let mut app = App::new_for_test().expect("test app should build");
        assert_eq!(
            app.app_modal_overlay_hit(Position::new(0, 0)),
            ModalHit::Closed
        );

        tui_pane::open_favorites(&mut app);
        assert_eq!(
            app.app_modal_overlay_hit(Position::new(0, 0)),
            ModalHit::MissedRow
        );
    }
}

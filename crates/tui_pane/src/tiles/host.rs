//! What an app does to carry a [`TileGrid`] as its body: where the grid
//! is kept, the pane that registers it, and what a click on it does.

use std::fmt::Debug;
use std::marker::PhantomData;

use ratatui::layout::Position;

use super::grid::TileGrid;
use crate::AppContext;
use crate::CycleDirection;
use crate::Framework;
use crate::FrameworkHit;
use crate::FrameworkOverlayId;
use crate::Hittable;
use crate::InputContext;
use crate::Mode;
use crate::Pane;
use crate::dispatch_hit_test;

/// An app whose body is one [`TileGrid`], registered as one pane.
///
/// Clicks reach the grid through [`handle_tile_click`], which also needs
/// the app's [`InputContext`] to answer with [`TilePick`]: the grid's
/// [`Hittable`] impl for its pane, and
/// [`TilePick::from_framework_hit`] for the framework's hits.
pub trait TileGridHost: AppContext + 'static {
    /// What the app names each group's cell by.
    type TileId: Clone + Eq + Debug;

    /// The app pane id [`TileGridPane`] registers the grid under.
    const TILE_GRID_PANE: Self::AppPaneId;

    /// The app's grid, to focus and steer.
    fn tile_grid_mut(&mut self) -> &mut TileGrid<Self::TileId>;
}

/// Keymap host for the grid. It has no keys of its own, so `A`, the app
/// hosting it, registers it with
/// [`register_pane`](crate::KeymapBuilder::register_pane).
pub struct TileGridPane<A>(PhantomData<fn() -> A>);

impl<A: TileGridHost> Pane<A> for TileGridPane<A> {
    const APP_PANE_ID: A::AppPaneId = A::TILE_GRID_PANE;

    /// The grid is not a list. Its cells are walked by the app globals
    /// and by Tab, so the status line's navigation region has nothing
    /// to say about it, and `Static` is what keeps that region off.
    fn mode() -> fn(&A) -> Mode<A> { |_app| Mode::Static }

    /// Tab walks the tile grid rather than the framework's pane cycle.
    /// The app registers one pane and puts every command in a cell
    /// inside it, so the cells are what a developer means by "the next
    /// one" -- and the step never falls through, because there is no
    /// second pane behind the grid to fall through to.
    fn cycle_step() -> Option<fn(&mut A, CycleDirection) -> bool> {
        Some(|app, direction| app.tile_grid_mut().cycle_focus(direction))
    }
}

/// What a click on a tile grid app found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TilePick {
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

impl TilePick {
    /// What a framework hit picks, for
    /// [`InputContext::map_framework_hit`].
    ///
    /// Toasts and overlay chrome are framework surfaces. An overlay row
    /// is the only framework hit that becomes an app action; every
    /// other hit has already been absorbed.
    #[must_use]
    pub const fn from_framework_hit(hit: FrameworkHit) -> Option<Self> {
        match hit {
            FrameworkHit::Overlay { id, row } => Some(Self::OverlayRow { id, row }),
            FrameworkHit::Toast(_) | FrameworkHit::ModalMissed => None,
        }
    }
}

impl<Id: Clone + Eq + Debug> Hittable<TilePick> for TileGrid<Id> {
    fn hit_test_at(&self, pos: Position) -> Option<TilePick> {
        self.cell_at(pos).map(TilePick::Cell)
    }
}

/// Act on the click at `position`, for
/// [`TerminalApp::click`](crate::TerminalApp::click).
///
/// Clicking a cell is how focus moves without the arrow keys, and it
/// reads the same way: the cell lights up and stays lit until something
/// takes it out of the grid. Clicking a row of an open framework
/// overlay selects it.
pub fn handle_tile_click<A>(app: &mut A, position: Position)
where
    A: TileGridHost + InputContext<Target = TilePick>,
{
    match dispatch_hit_test(app, position) {
        Some(TilePick::Cell(index)) => app.tile_grid_mut().focus_cell(index),
        Some(TilePick::OverlayRow { id, row }) => {
            select_framework_overlay_row(app.framework_mut(), id, row);
        },
        None => (),
    }
}

/// Move a framework overlay's selection to the row that was clicked.
fn select_framework_overlay_row<Ctx: AppContext>(
    framework: &mut Framework<Ctx>,
    id: FrameworkOverlayId,
    row: usize,
) {
    match id {
        FrameworkOverlayId::Settings => framework.settings_pane.select_row(row),
        FrameworkOverlayId::Keymap => framework.keymap_pane.viewport_mut().set_pos(row),
        FrameworkOverlayId::GlobalShortcuts => {
            framework.global_shortcuts_pane.viewport_mut().set_pos(row);
        },
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::FocusedPane;
    use crate::HitTestRegistry;
    use crate::ModalHit;
    use crate::NoToastAction;
    use crate::TABLE_CELL;
    use crate::TileAction;
    use crate::TileContent;
    use crate::ToastHit;
    use crate::ToastId;
    use crate::Viewport;

    /// The rows the first column holds before a second one opens.
    const INITIAL_ROWS: usize = 4;
    /// Width of the rect the test grid is laid out in.
    const WIDTH: u16 = 80;
    /// Height of the rect the test grid is laid out in.
    const HEIGHT: u16 = 24;
    /// A row index past the first, for the overlay selection tests.
    const ROW: usize = 3;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Grid,
    }

    /// Every pane a click can land on, top of the stack first.
    const Z_ORDER: [TestPaneId; 1] = [TestPaneId::Grid];

    /// Whether the test app's own modal is open over the grid.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Modal {
        Open,
        Closed,
    }

    struct TestApp {
        framework: Framework<Self>,
        grid:      TileGrid<u8>,
        modal:     Modal,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }

        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    impl TileGridHost for TestApp {
        type TileId = u8;

        const TILE_GRID_PANE: TestPaneId = TestPaneId::Grid;

        fn tile_grid_mut(&mut self) -> &mut TileGrid<u8> { &mut self.grid }
    }

    impl HitTestRegistry for TestApp {
        type PaneId = TestPaneId;
        type Target = TilePick;

        fn z_order() -> &'static [TestPaneId] { &Z_ORDER }

        fn pane(&self, id: TestPaneId) -> Option<&dyn Hittable<TilePick>> {
            match id {
                TestPaneId::Grid => Some(&self.grid),
            }
        }

        fn viewport_mut(&mut self, _: TestPaneId) -> Option<&mut Viewport> { None }
    }

    impl InputContext for TestApp {
        fn framework_hit(&self, pos: Position) -> Option<FrameworkHit> {
            self.framework.hit_test_at(pos)
        }

        fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<TilePick> {
            match self.modal {
                Modal::Open => ModalHit::MissedRow,
                Modal::Closed => ModalHit::Closed,
            }
        }

        fn map_framework_hit(&self, hit: FrameworkHit) -> Option<TilePick> {
            TilePick::from_framework_hit(hit)
        }
    }

    /// The rect the test grid is laid out in.
    const fn area() -> Rect { Rect::new(0, 0, WIDTH, HEIGHT) }

    /// An app whose grid holds the summary and one empty cell, settled.
    fn two_cell_app() -> TestApp {
        let mut grid = TileGrid::new();
        grid.set_layout(area(), INITIAL_ROWS);
        grid.apply(TileAction::Add, INITIAL_ROWS);
        grid.settle_for_test();
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Grid)),
            grid,
            modal: Modal::Closed,
        }
    }

    /// What the cell holding the focus ring shows.
    fn focused(app: &TestApp) -> TileContent<u8> {
        app.grid
            .placements(area(), INITIAL_ROWS)
            .into_iter()
            .find(|placement| placement.frame.is_focused())
            .map(|placement| placement.content)
            .expect("one cell holds the focus ring")
    }

    /// The middle of the cell showing `content`.
    fn middle_of(app: &TestApp, content: &TileContent<u8>) -> Position {
        let rect = app
            .grid
            .placements(area(), INITIAL_ROWS)
            .into_iter()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.rect())
            .expect("the grid draws the cell");
        Position::new(rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    #[test]
    fn a_click_on_a_cell_focuses_it() {
        let mut app = two_cell_app();
        assert_eq!(focused(&app), TileContent::Summary);

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        handle_tile_click(&mut app, cell);

        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
    }

    #[test]
    fn an_open_app_modal_absorbs_a_click_on_a_cell() {
        let mut app = two_cell_app();
        app.modal = Modal::Open;

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        handle_tile_click(&mut app, cell);

        assert_eq!(focused(&app), TileContent::Summary);
    }

    #[test]
    fn only_an_overlay_row_becomes_a_pick() {
        let cases = [
            (
                FrameworkHit::Overlay {
                    id:  FrameworkOverlayId::Keymap,
                    row: ROW,
                },
                Some(TilePick::OverlayRow {
                    id:  FrameworkOverlayId::Keymap,
                    row: ROW,
                }),
            ),
            (FrameworkHit::Toast(ToastHit::Close(ToastId(0))), None),
            (FrameworkHit::Toast(ToastHit::Card(ToastId(0))), None),
            (FrameworkHit::ModalMissed, None),
        ];
        for (hit, pick) in cases {
            assert_eq!(TilePick::from_framework_hit(hit), pick, "{hit:?}");
        }
    }

    #[test]
    fn an_overlay_row_pick_selects_that_row_in_each_overlay() {
        for id in [
            FrameworkOverlayId::Settings,
            FrameworkOverlayId::Keymap,
            FrameworkOverlayId::GlobalShortcuts,
        ] {
            let mut app = two_cell_app();
            select_framework_overlay_row(&mut app.framework, id, ROW);

            let viewport = match id {
                FrameworkOverlayId::Settings => app.framework.settings_pane.viewport(),
                FrameworkOverlayId::Keymap => app.framework.keymap_pane.viewport(),
                FrameworkOverlayId::GlobalShortcuts => {
                    app.framework.global_shortcuts_pane.viewport()
                },
            };
            assert_eq!(viewport.pos(), ROW, "{id:?}");
        }
    }

    #[test]
    fn the_grid_pane_is_static() {
        let app = two_cell_app();
        let mode = <TileGridPane<TestApp> as Pane<TestApp>>::mode();
        assert!(matches!(mode(&app), Mode::Static));
    }

    #[test]
    fn tab_walks_the_cells_and_wraps() {
        let mut app = two_cell_app();
        let step = <TileGridPane<TestApp> as Pane<TestApp>>::cycle_step()
            .expect("the grid pane takes Tab");

        assert!(step(&mut app, CycleDirection::Next));
        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
        assert!(step(&mut app, CycleDirection::Next));
        assert_eq!(focused(&app), TileContent::Summary);
        assert!(step(&mut app, CycleDirection::Prev));
        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
    }
}

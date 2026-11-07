//! What an app does to carry a [`TileGrid`] as its body: where the grid
//! is kept, the pane that registers it, and what a click on it does.

use std::fmt::Debug;
use std::marker::PhantomData;

use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::draw::content_area;
use super::grid::CellHit;
use super::grid::TileContent;
use super::grid::TileGrid;
use crate::AppContext;
use crate::CycleDirection;
use crate::Framework;
use crate::FrameworkHit;
use crate::FrameworkOverlayId;
use crate::Hittable;
use crate::InputContext;
use crate::Mode;
use crate::OverlayWheel;
use crate::Pane;
use crate::ScrollDirection;
use crate::SettingsHost;
use crate::dispatch_hit_test;
use crate::wheel_framework_overlay;

/// A terminal cell position relative to a tile's content origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellPoint {
    /// Horizontal cell offset within the content.
    pub column: u16,
    /// Vertical cell offset within the content.
    pub row:    u16,
}

/// A content pointer position resolved before focus changes the grid layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellPointer<Id> {
    /// The current grid index of the painted cell.
    pub index:   usize,
    /// The content assigned to that cell.
    pub content: TileContent<Id>,
    /// The content rectangle in terminal coordinates.
    pub area:    Rect,
    /// The pointer position relative to the content origin.
    pub point:   CellPoint,
}

/// A painted pointer target resolved before tile focus changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellPress<Id> {
    /// A press in a tile with content.
    Content {
        /// The source content coordinates.
        click:    CellPointer<Id>,
        /// The position in the painted frame.
        position: Position,
    },
    /// A press on the title row inside its corner cells.
    Title {
        /// The current grid index.
        index:    usize,
        /// The content assigned to the tile.
        content:  TileContent<Id>,
        /// The position in the painted frame.
        position: Position,
    },
    /// A press in an empty tile's content allocation.
    EmptyContent {
        /// The source content coordinates.
        click:    CellPointer<Id>,
        /// The position in the painted frame.
        position: Position,
    },
    /// A border or reserved readout row.
    Chrome,
    /// A painted tile that no longer owns a live slot.
    Retired,
}

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

    /// Handle a wheel step inside a cell without moving tile focus.
    fn cell_scrolled(&mut self, _cell: CellPointer<Self::TileId>, _direction: ScrollDirection) {}

    /// Handle a painted press after focus moves to a current tile.
    fn cell_pressed(&mut self, press: CellPress<Self::TileId>) {
        match press {
            CellPress::Content { click, .. } | CellPress::EmptyContent { click, .. } => {
                self.cell_clicked(click);
            },
            CellPress::Title { .. } | CellPress::Chrome | CellPress::Retired => {},
        }
    }

    /// Handle a left press inside a focused cell's content, after focus moves.
    fn cell_clicked(&mut self, _click: CellPointer<Self::TileId>) {}
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
    /// A departing cell is still painted but no longer owns a live slot.
    RetiredCell,
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
        match self.cell_hit(pos) {
            CellHit::Current { index, .. } => Some(TilePick::Cell(index)),
            CellHit::Retired => Some(TilePick::RetiredCell),
            CellHit::Missed => None,
        }
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
        Some(TilePick::Cell(index)) => {
            let press = painted_press(app.tile_grid_mut().cell_hit(position), position);
            app.tile_grid_mut().focus_cell(index);
            match press {
                CellPress::Chrome | CellPress::Retired => {},
                press => app.cell_pressed(press),
            }
        },
        Some(TilePick::OverlayRow { id, row }) => {
            select_framework_overlay_row(app.framework_mut(), id, row);
        },
        Some(TilePick::RetiredCell) | None => (),
    }
}

/// Forward a wheel step to the cell under the pointer, respecting modal hits.
pub fn handle_tile_scroll<A>(app: &mut A, position: Position, direction: ScrollDirection)
where
    A: TileGridHost + InputContext<Target = TilePick> + SettingsHost,
{
    if wheel_framework_overlay(app, position, direction) == OverlayWheel::Consumed {
        return;
    }
    if let Some(TilePick::Cell(_)) = dispatch_hit_test(app, position)
        && let Some(click) = content_click(app.tile_grid_mut().cell_hit(position), position)
    {
        app.cell_scrolled(click, direction);
    }
}

/// Translate a wheel position through the same allocation as a press.
fn content_click<Id>(hit: CellHit<Id>, position: Position) -> Option<CellPointer<Id>> {
    match painted_press(hit, position) {
        CellPress::Content { click, .. } | CellPress::EmptyContent { click, .. } => Some(click),
        CellPress::Title { .. } | CellPress::Chrome | CellPress::Retired => None,
    }
}

/// Resolve a painted press before focus can change the grid allocation.
fn painted_press<Id>(hit: CellHit<Id>, position: Position) -> CellPress<Id> {
    let (index, content, frame) = match hit {
        CellHit::Current {
            index,
            content,
            frame,
        } => (index, content, frame),
        CellHit::Retired => return CellPress::Retired,
        CellHit::Missed => return CellPress::Chrome,
    };
    let Ok(source_y) = u16::try_from(i32::from(position.y) - frame.shift()) else {
        return CellPress::Chrome;
    };
    let source = Position::new(position.x, source_y);
    let clip = frame.clip();
    if clip.contains(position)
        && i32::from(position.y) == i32::from(frame.rect().y) + frame.shift()
        && position.x > frame.rect().x
        && position.x < frame.rect().right().saturating_sub(1)
    {
        return CellPress::Title {
            index,
            content,
            position,
        };
    }
    let area = content_area(frame.inner());
    if !clip.contains(position) || !area.contains(source) {
        return CellPress::Chrome;
    }
    let click = CellPointer {
        index,
        content,
        area,
        point: CellPoint {
            column: source.x - area.x,
            row:    source.y - area.y,
        },
    };
    if matches!(click.content, TileContent::Empty(_)) {
        CellPress::EmptyContent { click, position }
    } else {
        CellPress::Content { click, position }
    }
}

/// The visible content and title allocations used by `painted_press`.
pub(super) fn painted_regions(frame: crate::PaneFrame) -> Vec<Rect> {
    let clip = frame.clip();
    let content = content_area(frame.inner());
    let shifted = |area: Rect| {
        let top = (i32::from(area.y) + frame.shift()).max(0);
        let bottom = (i32::from(area.bottom()) + frame.shift()).max(0);
        Rect::new(
            area.x,
            u16::try_from(top).unwrap_or(u16::MAX),
            area.width,
            u16::try_from(bottom - top).unwrap_or(u16::MAX),
        )
    };
    let content = shifted(content).intersection(clip);
    let title = shifted(Rect::new(
        frame.rect().x.saturating_add(1),
        frame.rect().y,
        frame.rect().width.saturating_sub(2),
        1,
    ))
    .intersection(clip);
    vec![content, title]
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
    use crate::TileFill;
    use crate::TileGrowth;
    use crate::ToastHit;
    use crate::ToastId;
    use crate::Viewport;

    /// How the test grid grows: four rows in the first column before a
    /// second one opens.
    const GROWTH: TileGrowth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
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
        grid.set_layout(area(), GROWTH);
        grid.apply(TileAction::Add, GROWTH);
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
            .placements(area(), GROWTH)
            .into_iter()
            .find(|placement| placement.frame.is_focused())
            .map(|placement| placement.content)
            .expect("one cell holds the focus ring")
    }

    /// The middle of the cell showing `content`.
    fn middle_of(app: &TestApp, content: &TileContent<u8>) -> Position {
        let rect = app
            .grid
            .placements(area(), GROWTH)
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

    #[test]
    fn tab_takes_no_step_while_the_summary_is_alone() {
        let mut app = two_cell_app();
        app.grid.set_min_tile_width(WIDTH / 2);
        app.grid.focus_cell(TABLE_CELL + 1);
        app.grid
            .set_layout(Rect::new(0, 0, WIDTH / 2 - 1, HEIGHT), GROWTH);
        let step = <TileGridPane<TestApp> as Pane<TestApp>>::cycle_step()
            .expect("the grid pane takes Tab");

        assert!(!step(&mut app, CycleDirection::Next));
        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod host_tests;

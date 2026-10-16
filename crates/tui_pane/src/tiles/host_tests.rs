//! A cell click reaches its host only inside the content it actually draws.

#![expect(clippy::expect_used, reason = "test grid placements must exist")]

use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::*;
use crate::FocusedPane;
use crate::HitTestRegistry;
use crate::ModalHit;
use crate::NoToastAction;
use crate::TileAction;
use crate::TileContent;
use crate::TileFill;
use crate::TileGrowth;
use crate::Viewport;
use crate::tiles::draw;

const GROWTH: TileGrowth = TileGrowth {
    initial_rows:  4,
    fill:          TileFill::Redistribute,
    widen_summary: false,
};
const AREA: Rect = Rect::new(7, 5, 100, 32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PaneId {
    Grid,
}

struct ClickHost {
    framework:           Framework<Self>,
    grid:                TileGrid<u8>,
    clicks:              Vec<CellClick<u8>>,
    focused_during_hook: Vec<TileContent<u8>>,
}

impl AppContext for ClickHost {
    type AppPaneId = PaneId;
    type ToastAction = NoToastAction;
    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

impl TileGridHost for ClickHost {
    type TileId = u8;
    const TILE_GRID_PANE: PaneId = PaneId::Grid;
    fn tile_grid_mut(&mut self) -> &mut TileGrid<u8> { &mut self.grid }
    fn cell_clicked(&mut self, click: CellClick<u8>) {
        self.clicks.push(click);
        let focused = self
            .grid
            .placements(AREA, GROWTH)
            .iter()
            .find(|placement| placement.frame.is_focused())
            .expect("focused frame")
            .content
            .clone();
        self.focused_during_hook.push(focused);
    }
}

impl HitTestRegistry for ClickHost {
    type PaneId = PaneId;
    type Target = TilePick;
    fn z_order() -> &'static [PaneId] { &[PaneId::Grid] }
    fn pane(&self, id: PaneId) -> Option<&dyn Hittable<TilePick>> {
        match id {
            PaneId::Grid => Some(&self.grid),
        }
    }
    fn viewport_mut(&mut self, _: PaneId) -> Option<&mut Viewport> { None }
}

impl InputContext for ClickHost {
    fn framework_hit(&self, pos: Position) -> Option<FrameworkHit> {
        self.framework.hit_test_at(pos)
    }
    fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<TilePick> { ModalHit::Closed }
    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<TilePick> {
        TilePick::from_framework_hit(hit)
    }
}

fn host() -> ClickHost {
    let mut grid = TileGrid::new();
    grid.set_layout(AREA, GROWTH);
    grid.apply(TileAction::Add, GROWTH);
    grid.focus_cell(1);
    grid.settle_for_test();
    ClickHost {
        framework: Framework::new(FocusedPane::App(PaneId::Grid)),
        grid,
        clicks: Vec::new(),
        focused_during_hook: Vec::new(),
    }
}

#[test]
fn content_click_passes_cell_local_coordinates_after_focusing_the_cell() {
    let mut app = host();
    let placements = app.grid.placements(AREA, GROWTH);
    let placement = placements
        .iter()
        .find(|placement| matches!(placement.content, TileContent::Empty(_)))
        .expect("empty cell");
    assert!(!placement.frame.is_focused(), "the press must move focus");
    let content = draw::content_area(placement.frame.inner());
    let point = CellPoint {
        column: 3,
        row:    2,
    };
    handle_tile_click(
        &mut app,
        Position::new(content.x + point.column, content.y + point.row),
    );
    assert_eq!(
        app.clicks,
        [CellClick {
            index: 2,
            content: placement.content.clone(),
            area: content,
            point
        }]
    );
    assert_eq!(
        app.focused_during_hook.as_slice(),
        std::slice::from_ref(&placement.content)
    );
}

#[test]
fn frame_borders_and_the_reserved_readout_do_not_call_the_content_hook() {
    let mut app = host();
    let placements = app.grid.placements(AREA, GROWTH);
    let placement = placements
        .iter()
        .find(|placement| matches!(placement.content, TileContent::Empty(_)))
        .expect("empty cell");
    let frame = placement.frame.rect();
    let content = draw::content_area(placement.frame.inner());
    for point in [
        Position::new(frame.x, frame.y),
        Position::new(frame.x + frame.width - 1, frame.y + 1),
        Position::new(content.x, content.y + content.height),
    ] {
        handle_tile_click(&mut app, point);
        assert!(app.clicks.is_empty(), "{point:?}");
    }
}

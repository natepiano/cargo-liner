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
use crate::TileGridContents;
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
    clicks:              Vec<CellPointer<u8>>,
    presses:             Vec<CellPress<u8>>,
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
    fn cell_pressed(&mut self, press: CellPress<u8>) {
        self.presses.push(press.clone());
        match press {
            CellPress::Content { click, .. } | CellPress::EmptyContent { click, .. } => {
                self.cell_clicked(click);
            },
            CellPress::Title { .. } | CellPress::Chrome | CellPress::Retired => {},
        }
    }
    fn cell_clicked(&mut self, click: CellPointer<u8>) {
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
        presses: Vec::new(),
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
        [CellPointer {
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

#[test]
fn title_presses_reach_the_host_without_calling_the_content_hook() {
    let mut app = host();
    let placement = app
        .grid
        .placements(AREA, GROWTH)
        .into_iter()
        .find(|placement| matches!(placement.content, TileContent::Empty(_)))
        .expect("empty cell");
    let frame = placement.frame.rect();
    let position = Position::new(frame.x + 2, frame.y);
    handle_tile_click(&mut app, position);
    assert_eq!(
        app.presses,
        [CellPress::Title {
            index: 2,
            content: placement.content,
            position
        }]
    );
    assert_eq!(app.clicks, Vec::<CellPointer<u8>>::new());
}

#[test]
fn borders_and_the_foot_readout_do_not_forward_word_targets() {
    let mut app = host();
    let placement = app
        .grid
        .placements(AREA, GROWTH)
        .into_iter()
        .find(|placement| matches!(placement.content, TileContent::Empty(_)))
        .expect("empty cell");
    let frame = placement.frame.rect();
    let content = draw::content_area(placement.frame.inner());
    for position in [
        Position::new(frame.x, frame.y),
        Position::new(content.x, content.bottom()),
    ] {
        handle_tile_click(&mut app, position);
        assert_eq!(app.presses, Vec::<CellPress<u8>>::new());
    }
}

#[test]
fn retired_content_and_titles_do_not_change_focus_or_reach_the_host() {
    let mut app = host();
    let placement = app
        .grid
        .placements(AREA, GROWTH)
        .into_iter()
        .find(|placement| matches!(placement.content, TileContent::Empty(_)))
        .expect("empty cell");
    app.grid.focus_cell(2);
    app.grid.settle_for_test();
    let drawing = app.grid.drawing(AREA, GROWTH);
    app.grid
        .record_painted_body(drawing, TileGridContents::Shown, Vec::new());
    app.grid.apply(TileAction::Remove, GROWTH);
    let focus = app.grid.focused_cell();
    let frame = placement.frame.rect();
    let content = draw::content_area(placement.frame.inner());
    for position in [
        Position::new(frame.x + 2, frame.y),
        Position::new(content.x + 2, content.y),
    ] {
        handle_tile_click(&mut app, position);
        assert_eq!(app.grid.focused_cell(), focus);
        assert_eq!(app.presses, Vec::<CellPress<u8>>::new());
        assert_eq!(app.clicks, Vec::<CellPointer<u8>>::new());
    }
}

#[test]
fn shifted_clipped_frames_keep_content_and_title_targets_separate() {
    let rect = Rect::new(7, 5, 40, 15);
    let clip = Rect::new(7, 8, 40, 12);
    let frame = crate::PaneFrame::shifted(rect, -2, clip);
    let position = Position::new(10, clip.y);
    let hit = |frame| CellHit::Current {
        index: 1,
        content: TileContent::Group(4_u8),
        frame,
    };
    assert!(matches!(
        painted_press(hit(frame), position),
        CellPress::Content { .. }
    ));
    assert!(
        painted_regions(frame)
            .iter()
            .any(|area| area.contains(position))
    );
    let title_frame = crate::PaneFrame::shifted(rect, 4, clip);
    let title = Position::new(10, rect.y + 4);
    assert!(matches!(
        painted_press(hit(title_frame), title),
        CellPress::Title { .. }
    ));
    assert!(
        painted_regions(title_frame)
            .iter()
            .any(|area| area.contains(title))
    );
    assert_eq!(painted_regions(frame)[1].height, 0);
}

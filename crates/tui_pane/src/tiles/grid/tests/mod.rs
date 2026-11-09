mod motion;
mod motion_rules;
mod summary_depth;

use std::ops::RangeInclusive;

use ratatui::buffer::Buffer;
use ratatui::layout::Margin;
use ratatui::style::Color;

use super::draw::TileMeasurements;
use super::*;
use crate::ColorMode;
use crate::TileGridContents;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::SEVEN_CELL_MOVING_ROW_PROBE_COUNT;
use crate::tiles::constants::TILE_BORDER_ROWS;
use crate::tiles::constants::TILE_ROWS_READOUT_HEIGHT;

/// Width of the rect the placement tests lay their grids out in.
pub(super) const TEST_WIDTH: u16 = 80;

/// Height of the rect the placement tests lay their grids out in.
pub(super) const TEST_HEIGHT: u16 = 40;

/// App-level floor used by the tile-width tests.
pub(super) const TEST_TILE_WIDTH: u16 = 40;

/// The rect the placement tests lay their grids out in.
pub(super) const fn test_area() -> Rect { Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT) }

/// The rows a cell drawing `rows` of content asks for under the
/// default settings.
pub(super) fn demanded_rows(rows: usize) -> u16 {
    TileSettings::default().demanded_rows(rows).stepped_framed()
}

/// One demand whose exact and stepped framed rows agree.
pub(super) const fn row_demand(framed: u16) -> CellRowDemand { CellRowDemand::uniform(framed) }

/// Demands whose exact and stepped framed rows agree.
pub(super) fn row_demands(framed: &[u16]) -> Vec<CellRowDemand> {
    framed.iter().copied().map(row_demand).collect()
}

/// `count` cells with nothing to show, which every column divides
/// evenly -- the geometry and motion tests are written against it.
pub(super) fn even(count: usize) -> HeldCellLayout {
    HeldCellLayout {
        rows:          vec![row_demand(demanded_rows(0)); count],
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    }
}

/// What one scan of `ids` demands, every cell asking for the floor
/// so the arrangement is all that moves.
pub(super) fn quiet<Id: Clone>(ids: &[Id]) -> TileDemands<Id> {
    TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids
            .iter()
            .map(|id| TileDemand {
                id:   id.clone(),
                rows: 0,
            })
            .collect(),
    }
}

/// A tally with one entry per cell of [`test_area`].
pub(super) fn blank_tally() -> Vec<u32> {
    vec![0; usize::from(TEST_WIDTH) * usize::from(TEST_HEIGHT)]
}

/// Count every cell of [`test_area`] that `rect` covers.
pub(super) fn paint(tally: &mut [u32], rect: Rect) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            let index = usize::from(y) * usize::from(TEST_WIDTH) + usize::from(x);
            if let Some(cell) = tally.get_mut(index) {
                *cell += 1;
            }
        }
    }
}

/// How a test grid grows at `initial_rows` under
/// [`TileFill::AddNew`].
pub(super) const fn add_new(initial_rows: usize) -> TileGrowth {
    TileGrowth {
        initial_rows,
        fill: TileFill::AddNew,
        widen_summary: false,
    }
}

/// How a test grid grows at `initial_rows` under
/// [`TileFill::Redistribute`], the fill an app gets by default.
pub(super) const fn redistribute(initial_rows: usize) -> TileGrowth {
    TileGrowth {
        initial_rows,
        fill: TileFill::Redistribute,
        widen_summary: false,
    }
}

/// How a test grid grows at `initial_rows` with the summary widening.
pub(super) const fn widening(initial_rows: usize) -> TileGrowth {
    TileGrowth {
        widen_summary: true,
        ..redistribute(initial_rows)
    }
}

/// Paint the wide, short right band that is admissible only at the old floor.
pub(super) fn painted_widened_floor_fixture() -> (TileGrid<u32>, Rect, TileGrowth, TileDemands<u32>)
{
    let area = Rect::new(0, 0, 80, 47);
    let growth = widening(4);
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(3);
    grid.set_layout(area, growth);
    let demands = TileDemands {
        summary:       33,
        summary_width: 80,
        groups:        (1..=9).map(|id| TileDemand { id, rows: 3 }).collect(),
    };
    grid.sync(&demands, growth);
    grid.settle_for_test();
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |_, content| content.height,
        |_, content| content.height,
    );
    assert!(
        matches!(&grid.last_settled_geometry, LastSettledCellGeometry::Drawn { grid, .. } if grid.span == 2)
    );
    (grid, area, growth, demands)
}

pub(super) fn demand_resize_motion_fixture() -> (TileGrid<u32>, Rect, TileGrowth, TileDemands<u32>)
{
    let area = Rect::new(0, 0, 200, 50);
    let growth = redistribute(4);
    let mut grid = TileGrid::<u32>::new();
    grid.set_layout(area, growth);
    let mut demands = TileDemands {
        summary:       3,
        summary_width: 0,
        groups:        (1..=3).map(|id| TileDemand { id, rows: 3 }).collect(),
    };
    grid.sync(&demands, growth);
    grid.settle_for_test();
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |_, area| area.height,
        |_, area| area.height,
    );
    demands.groups[0].rows = 30;
    grid.sync(&demands, growth);
    assert!(matches!(grid.motion, GridMotion::Moving(_)));
    (grid, area, growth, demands)
}

pub(super) fn assert_growth_keeps_groups_visible(
    grid: &TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
) {
    for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
        let drawing = grid.drawing_at(area, growth, progress);
        for id in 1..=3 {
            assert!(
                drawing.pieces.iter().any(|piece| {
                    piece.placement.content == TileContent::Group(id)
                        && piece.placement.frame.clip().height
                            >= grid.settings.min_tile_height.get()
                }),
                "growth change hides group {id} at progress {progress}"
            );
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum MotionLayoutChange {
    HeightFloor,
    GridArea,
}

/// Fill every motion cache and queue another endpoint before changing its inputs.
pub(super) fn assert_layout_change_discards_old_motion_geometry(change: MotionLayoutChange) {
    let (mut grid, area, growth, mut demands) = painted_widened_floor_fixture();
    demands.groups[0].rows = 12;
    grid.sync(&demands, growth);
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE / 2,
        |_, content| content.height,
        |_, content| content.height,
    );
    assert!(matches!(grid.motion, GridMotion::Moving(_)));
    assert_ne!(grid.frame_row_usage, [] as [FrameCellRowUsage<u32>; 0]);
    assert_ne!(grid.motion_heights.piece_heights, [] as [u16; 0]);
    grid.retain_measured_destination_for_test();
    assert!(grid.has_retargeted_destination_for_test());
    grid.apply(TileAction::Add, growth);
    assert!(!grid.pending.is_empty());
    let target = grid.target();
    let (area, floor) = match change {
        MotionLayoutChange::HeightFloor => {
            grid.set_min_tile_height(6);
            (area, 6)
        },
        MotionLayoutChange::GridArea => {
            let resized = Rect::new(2, 1, 80, 39);
            grid.set_draw_layout(resized, growth);
            (resized, 3)
        },
    };
    assert_eq!(
        grid.slots, target,
        "the requested cell order survives replanning"
    );
    assert!(grid.pending.is_empty());
    assert!(matches!(grid.motion, GridMotion::Settled));
    assert_eq!(grid.frame_row_usage, [] as [FrameCellRowUsage<u32>; 0]);
    assert_eq!(grid.motion_heights.piece_heights, [] as [u16; 0]);
    assert!(!grid.has_retargeted_destination_for_test());
    assert!(matches!(
        grid.last_settled_geometry,
        LastSettledCellGeometry::NoCellsFrame
    ));
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE / 2,
        |_, content| content.height,
        |_, content| content.height,
    );
    for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
        let drawing = grid.drawing_at(area, growth, progress);
        assert_ne!(drawing.pieces, [] as [TilePiece<u32>; 0]);
        for piece in &drawing.pieces {
            let frame = piece.placement.frame.clip();
            assert!(frame.height >= floor, "changed layout floor: {frame:?}");
            assert_eq!(frame.intersection(area), frame);
        }
    }
}

/// Both fills, for the rules that hold whichever one is set.
pub(super) const FILLS: [TileFill; 2] = [TileFill::AddNew, TileFill::Redistribute];

/// What one scan of `(id, rows)` pairs demands, so a test can say
/// which cells are busy and which are idle.
pub(super) fn busy(groups: &[(u32, usize)]) -> TileDemands<u32> {
    TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        groups
            .iter()
            .map(|&(id, rows)| TileDemand { id, rows })
            .collect(),
    }
}

/// The rows one column hands each of its cells, top to bottom.
///
/// The allocation rather than the rects it becomes: neighbouring
/// cells share a border line, so the rects overlap by one and no
/// longer add up to the column.
pub(super) fn column_rows(wants: &[u16], focused: ColumnFocus) -> Vec<u16> {
    let wants = row_demands(wants);
    shares(
        &wants,
        TEST_HEIGHT,
        ColumnHead::Cell,
        focused,
        MIN_TILE_HEIGHT,
    )
    .into_iter()
    .map(|size| match size {
        PaneAxisSize::Fixed(rows) => rows,
        PaneAxisSize::Fill(weight) => weight,
    })
    .collect()
}

/// Unused rows from the measured five-cell column at each considered height.
pub(super) const fn measured_column_unused(index: usize, rows: u16, _: RowUsePurpose) -> u16 {
    match (index, rows) {
        (0 | 2 | 3, 6) => 1,
        _ => 0,
    }
}

/// One cell as the wrapping tests draw it; nothing about the motion
/// depends on what it holds.
pub(super) fn drawn() -> CellAppearance<u32> {
    CellAppearance {
        identity: CellIdentity::Summary,
        content:  TileContent::Summary,
        focused:  false,
    }
}

/// A grid the tests can grow without a terminal under it.
pub(super) fn seeded_grid<Id: Clone + Eq + Debug>() -> TileGrid<Id> {
    let mut grid = TileGrid::new();
    grid.set_layout(test_area(), redistribute(4));
    grid
}

/// Twenty-four exact points including both ends of a motion.
pub(super) fn motion_steps() -> impl Iterator<Item = u32> {
    (0..24).map(|step| step * PROGRESS_SCALE / 23)
}

/// A key fixture whose summary needs two positions before `+` and one
/// after it. The command remains short of its exact row demand throughout.
pub(super) fn depth_changing_key_grid() -> (TileGrid<u32>, Rect, TileGrowth) {
    let area = test_area();
    let growth = add_new(4);
    let demands = TileDemands {
        summary:       usize::from(TEST_HEIGHT) * 3 / 4,
        summary_width: 0,
        groups:        vec![TileDemand {
            id:   1,
            rows: usize::from(TEST_HEIGHT),
        }],
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.sync(&demands, growth);
    grid.settle_for_test();
    assert_eq!(grid.depth, 2);
    (grid, area, growth)
}

/// Demands that keep every stepped fixture's content incomplete.
pub(super) fn tall_demands(ids: &[u32]) -> TileDemands<u32> {
    let rows = usize::from(TEST_HEIGHT).saturating_mul(2);
    TileDemands {
        summary:       rows,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows }).collect(),
    }
}

/// A third resident enters one column whose other pieces both shrink.
pub(super) fn stepped_height_motion() -> (TileGrid<u32>, Rect, TileGrowth) {
    let area = test_area();
    let growth = redistribute(usize::from(TEST_HEIGHT));
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.sync(&tall_demands(&[1]), growth);
    grid.settle_for_test();
    grid.collapse_queued_changes_for_test();
    grid.sync(&tall_demands(&[1, 2]), growth);
    (grid, area, growth)
}

/// One single-column motion with exact framed endpoint heights.
pub(super) fn exact_height_motion(
    before: &[u16],
    after: &[u16],
) -> (TileGrid<u32>, Rect, TileGrowth) {
    let cells = before.len();
    let height = before
        .iter()
        .copied()
        .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
        .unwrap_or_default();
    assert_eq!(
        height,
        after
            .iter()
            .copied()
            .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
            .unwrap_or_default()
    );
    let area = Rect::new(0, 0, TEST_WIDTH, height);
    let growth = redistribute(cells);
    let slots = (1..u32::try_from(cells).unwrap_or(u32::MAX))
        .map(Slot::Group)
        .collect::<Vec<_>>();
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.slots = slots.clone();
    grid.held = HeldCellLayout {
        rows:          row_demands(after),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let before = HeldCellLayout {
        rows:          row_demands(before),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let source = Grid::new(area, &before, growth, &grid.settings);
    grid.motion = GridMotion::Moving(Transition {
        from:         slots,
        held:         before,
        source:       Box::new(source),
        started:      Instant::now(),
        millis:       1,
        depth_status: DestinationDepthStatus::Resolved,
    });
    (grid, area, growth)
}

/// One single-column add or remove motion with exact endpoint heights.
pub(super) fn exact_slot_height_motion(
    before: &[u16],
    after: &[u16],
    before_slots: Vec<Slot<u32>>,
    after_slots: Vec<Slot<u32>>,
) -> (TileGrid<u32>, Rect, TileGrowth) {
    let source_height = before
        .iter()
        .copied()
        .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
        .unwrap_or_default();
    let destination_height = after
        .iter()
        .copied()
        .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
        .unwrap_or_default();
    assert_eq!(source_height, destination_height);
    assert_eq!(before.len(), before_slots.len().saturating_add(1));
    assert_eq!(after.len(), after_slots.len().saturating_add(1));
    let area = Rect::new(0, 0, TEST_WIDTH, source_height);
    let growth = redistribute(before.len().max(after.len()));
    let source_held = HeldCellLayout {
        rows:          row_demands(before),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let destination_held = HeldCellLayout {
        rows:          row_demands(after),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let source = Grid::new(area, &source_held, growth, &TileSettings::default());
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.slots = after_slots;
    grid.held = destination_held;
    grid.motion = GridMotion::Moving(Transition {
        from:         before_slots,
        held:         source_held,
        source:       Box::new(source),
        started:      Instant::now(),
        millis:       1,
        depth_status: DestinationDepthStatus::Resolved,
    });
    (grid, area, growth)
}

/// Draw the exact-height fixture with every candidate height filled.
pub(super) fn filled_height_drawing(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
    raw: u32,
) -> TileDrawing<u32> {
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |_, area| area.height,
        |_, area| area.height,
    );
    grid.drawing_at(area, growth, raw)
}

/// Record one settled frame and return its framed height by content.
pub(super) fn recorded_settled_heights(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
) -> Vec<(TileContent<u32>, u16)> {
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |content, area| match content {
            TileContent::Empty(_) => 0,
            TileContent::Summary | TileContent::Group(_) => area.height,
        },
        |content, area| match content {
            TileContent::Empty(_) => 0,
            TileContent::Summary | TileContent::Group(_) => area.height,
        },
    );
    grid.drawing_at(area, growth, PROGRESS_SCALE)
        .placements()
        .map(|placement| (placement.content.clone(), placement.frame.clip().height))
        .collect()
}

/// One content's framed height in a drawing.
pub(super) fn piece_height(drawing: &TileDrawing<u32>, content: &TileContent<u32>) -> Option<u16> {
    drawing
        .placements()
        .find(|placement| &placement.content == content)
        .map(|placement| placement.frame.clip().height)
}

/// Draw and inspect a motion whose contents fill selected framed heights.
pub(super) fn assert_snapped_heights_move_one_way(
    before: &[u16],
    after: &[u16],
    valid_heights: &[&[u16]],
) {
    let (mut grid, area, growth) = exact_height_motion(before, after);
    let mut piece_heights = vec![Vec::new(); before.len()];
    for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| {
                if area.height == u16::MAX {
                    return area.height;
                }
                let index = match content {
                    TileContent::Summary => 0,
                    TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
                    TileContent::Empty(_) => return area.height,
                };
                let framed = area
                    .height
                    .saturating_add(TILE_BORDER_ROWS)
                    .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                if valid_heights[index].contains(&framed) {
                    area.height
                } else {
                    area.height.saturating_sub(1)
                }
            },
            |_, area| area.height,
        );
        let drawing = grid.drawing_at(area, growth, raw);
        for (index, placement) in drawing.placements().enumerate() {
            let height = placement.frame.clip().height;
            assert!(
                valid_heights[index].contains(&height),
                "piece {index} uses unfilled height {height} at {raw}"
            );
            piece_heights[index].push(height);
        }
    }
    for (index, ((&start, &end), heights)) in
        before.iter().zip(after).zip(piece_heights).enumerate()
    {
        assert_eq!(heights.first().copied(), Some(start));
        assert_eq!(heights.last().copied(), Some(end));
        let one_way = if start <= end {
            heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(one_way, "piece {index} reverses through {heights:?}");
    }
}

/// Both sides of every individual or aggregate integer height transfer.
pub(super) fn single_column_height_steps(drawing: &TileDrawing<u32>) -> Vec<u32> {
    let mut changes = Vec::new();
    let (mut grown, mut shrunk) = (0_u16, 0_u16);
    for piece in &drawing.pieces {
        if let PieceHeightSnap::Search { start, end } = piece.height_snap {
            changes.push(start.abs_diff(end));
            grown = grown.saturating_add(end.saturating_sub(start));
            shrunk = shrunk.saturating_add(start.saturating_sub(end));
        }
    }
    changes.extend([grown, shrunk]);
    changes.sort_unstable();
    changes.dedup();
    let mut prior = vec![0; changes.len()];
    let mut steps = vec![0, PROGRESS_SCALE];
    for raw in 1..=PROGRESS_SCALE {
        let heights = changes
            .iter()
            .map(|&change| lerp(0, change, eased(raw)))
            .collect::<Vec<_>>();
        if heights != prior {
            steps.extend([raw.saturating_sub(1), raw]);
            prior = heights;
        }
    }
    steps.sort_unstable();
    steps.dedup();
    steps
}

/// Draw one fixed progress point after probing a two-row content step.
pub(super) fn stepped_height_drawing(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
    raw: u32,
    calls: &mut usize,
) -> TileDrawing<u32> {
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |content, area| {
            *calls = calls.saturating_add(1);
            let fills = match content {
                TileContent::Summary | TileContent::Group(_) => area.height % 2 == 1,
                TileContent::Empty(_) => true,
            };
            if fills {
                area.height
            } else {
                area.height.saturating_sub(1)
            }
        },
        |_, area| area.height,
    );
    grid.drawing_at(area, growth, raw)
}

/// Whether the two-row fixture fills this framed height.
pub(super) fn stepped_height_fills(content: &TileContent<u32>, rect: Rect) -> bool {
    let area = draw::content_area(frame_inner(rect));
    area.is_empty()
        || match content {
            TileContent::Summary | TileContent::Group(_) => area.height % 2 == 1,
            TileContent::Empty(_) => true,
        }
}

/// A settled four-cell column beginning its two-column opening.
pub(super) fn opening_column() -> TileGrid<u32> {
    let growth = redistribute(4);
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[1, 2, 3]), growth);
    grid.settle_for_test();
    grid.add(growth);
    grid
}

/// A public sync from `before` to `after`, paused on its first step.
pub(super) fn synced_motion(before: &[u32], after: &[u32], growth: TileGrowth) -> TileGrid<u32> {
    let mut grid = seeded_grid();
    grid.set_layout(test_area(), growth);
    grid.sync(&quiet(before), growth);
    grid.settle_for_test();
    grid.sync(&quiet(after), growth);
    grid
}

/// What each cell after the summary is showing, settled.
pub(super) fn shown<Id: Clone + Eq + Debug>(grid: &TileGrid<Id>) -> Vec<TileContent<Id>> {
    cells(&grid.slots)
        .into_iter()
        .skip(1)
        .map(|(content, _)| content)
        .collect()
}

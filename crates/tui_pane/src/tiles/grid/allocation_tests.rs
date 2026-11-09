use std::cell::Cell;
use std::collections::BTreeSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Margin;
use ratatui::style::Color;

use super::*;
use crate::ColorMode;
use crate::TileGridContents;
use crate::tiles::constants::LARGE_CONTENT_TEST_CELLS;
use crate::tiles::constants::LARGE_CONTENT_TEST_HEIGHT;
use crate::tiles::constants::LARGE_CONTENT_TEST_INITIAL_ROWS;
use crate::tiles::constants::LARGE_CONTENT_TEST_ROW_STEP;
use crate::tiles::constants::LARGE_CONTENT_TEST_ROWS;
use crate::tiles::constants::LARGE_CONTENT_TEST_WIDTHS;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::TILE_BORDER_ROWS;
use crate::tiles::constants::TILE_DEMAND_STEP;
use crate::tiles::constants::TILE_ROWS_READOUT_HEIGHT;
use crate::tiles::grid::tests::*;
use crate::tiles::grid::*;

#[test]
fn the_configured_tile_floor_fits_each_column_count_at_its_shared_width() {
    let growth = redistribute(1);
    let settings = TileSettings {
        min_tile_width: TEST_TILE_WIDTH,
        ..TileSettings::default()
    };
    for (count, width) in [(25, 196), (16, 157), (9, 118), (4, 79), (1, 40)] {
        let area = Rect::new(0, 0, width, u16::MAX);
        assert!(fits(area, count, growth, &settings), "{count} at {width}");
        assert!(
            !fits(
                Rect {
                    // No core-width column fits, even after reducing the column count.
                    width: TEST_TILE_WIDTH - 1,
                    ..area
                },
                count,
                growth,
                &settings,
            ),
            "{count} below the column floor"
        );
        let layout = match column_layout(count, 1, growth, area, &settings) {
            ColumnPlan::Columns(layout) => Some(layout),
            ColumnPlan::InsufficientWidth | ColumnPlan::InsufficientRows => None,
        }
        .expect("the positive threshold admits columns");
        let opened = Layout::horizontal(constraints_for_sizes(&fills(layout.columns.widths.len())))
            .split(area);
        assert!(
            opened
                .iter()
                .all(|&column| share_borders(column, area).width >= TEST_TILE_WIDTH)
        );
    }
}

#[test]
fn the_framework_floor_still_fits_three_columns_in_twenty_two() {
    let area = Rect::new(0, 0, 22, u16::MAX);
    assert!(fits(area, 9, redistribute(1), &TileSettings::default()));
}

#[test]
fn a_column_at_the_exact_floor_draws_no_piece_under_it() {
    let floor = MIN_TILE_HEIGHT.saturating_mul(2);
    let settings = TileSettings {
        min_tile_height: NonZeroU16::MIN.saturating_add(floor - 1),
        ..TileSettings::default()
    };
    for pieces in [2_u16, 3] {
        for extra in [0, 1] {
            let area = Rect::new(
                0,
                0,
                TEST_WIDTH,
                shared_run(pieces, floor).saturating_add(extra),
            );
            let grid = Grid::new(
                area,
                &even(usize::from(pieces)),
                add_new(usize::from(pieces)),
                &settings,
            );
            for cell in TABLE_CELL..TABLE_CELL + usize::from(pieces) {
                let rect = grid.cell(cell).expect("the column holds each piece");
                assert!(
                    rect.height >= floor,
                    "{pieces} pieces with {extra} extra rows gave cell {cell} {rect:?}"
                );
            }
        }
    }
}

#[test]
fn a_widened_summary_leaves_no_piece_under_the_floor() {
    let floor = MIN_TILE_HEIGHT.saturating_mul(2);
    let settings = TileSettings {
        min_tile_height: NonZeroU16::MIN.saturating_add(floor - 1),
        ..TileSettings::default()
    };
    let area = Rect::new(0, 0, TEST_WIDTH, shared_run(2, floor).saturating_mul(2));
    let held = HeldCellLayout {
        summary_span: 2,
        ..even(4)
    };
    let grid = Grid::new(area, &held, widening(3), &settings);

    assert_eq!(grid.span, 2);
    for cell in TABLE_CELL..TABLE_CELL + held.rows.len() {
        let rect = grid.cell(cell).expect("the widened grid holds each piece");
        assert!(
            rect.height >= floor,
            "cell {cell} is under the floor: {rect:?}"
        );
    }
}

#[test]
fn a_growth_change_during_demand_motion_keeps_every_group_visible() {
    let (mut grid, area, _, demands) = demand_resize_motion_fixture();
    let growth = redistribute(1);
    grid.sync(&demands, growth);
    grid.set_layout(area, growth);
    assert_growth_keeps_groups_visible(&grid, area, growth);
}

#[test]
fn a_layout_growth_change_replans_the_motion_endpoints() {
    let (mut grid, area, _, _) = demand_resize_motion_fixture();
    let growth = redistribute(1);
    grid.set_layout(area, growth);
    assert_growth_keeps_groups_visible(&grid, area, growth);
}

#[test]
fn a_floor_change_rechecks_the_painted_widened_summary_endpoint() {
    let (mut grid, area, growth, demands) = painted_widened_floor_fixture();
    grid.set_min_tile_height(6);
    grid.sync(&demands, growth);
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE / 2,
        |_, content| content.height,
        |_, content| content.height,
    );
    let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
    assert_ne!(drawing.pieces, [] as [TilePiece<u32>; 0]);
    for piece in &drawing.pieces {
        assert!(piece.placement.frame.clip().height >= 6);
    }
}

#[test]
fn a_floor_change_discards_queued_endpoints_and_measured_motion_heights() {
    assert_layout_change_discards_old_motion_geometry(MotionLayoutChange::HeightFloor);
}

#[test]
fn width_limited_columns_keep_the_exact_resident_capacity() {
    let area = Rect::new(0, 0, 80, 47);
    let settings = TileSettings {
        min_tile_width: 40,
        min_tile_height: NonZeroU16::MIN.saturating_add(6 - 1),
        ..TileSettings::default()
    };
    assert!(fits(area, 18, redistribute(4), &settings));
    assert!(!fits(area, 19, redistribute(4), &settings));
    assert!(!fits(
        Rect::new(0, 0, 40, u16::MAX),
        2,
        redistribute(4),
        &TileSettings {
            min_tile_height: NonZeroU16::MIN.saturating_add(u16::MAX - 1),
            ..settings
        }
    ));
}

#[test]
fn summary_redistribution_checks_the_remaining_resident_band() {
    let settings = TileSettings {
        min_tile_width: 40,
        min_tile_height: NonZeroU16::MIN.saturating_add(6 - 1),
        summary_row_allocation: SummaryRowAllocation::BeforeTiles,
        ..TileSettings::default()
    };
    let held = HeldCellLayout {
        rows: row_demands(&[40; 16]),
        summary_span: 2,
        ..even(16)
    };
    for (height, fits_summary) in [(47, false), (83, true)] {
        let area = Rect::new(0, 0, 80, height);
        let plan = match column_layout(16, 1, redistribute(4), area, &settings) {
            ColumnPlan::Columns(plan) => plan.for_summary(area, &held, &settings),
            unavailable => unavailable,
        };
        assert_eq!(matches!(plan, ColumnPlan::Columns(_)), fits_summary);
        if fits_summary {
            let grid = Grid::new(area, &held, redistribute(4), &settings);
            assert_eq!(grid.span, 2);
            assert_eq!(grid.resolved.panes.len(), 16);
            assert!(grid.resolved.panes.iter().all(|pane| pane.area.height >= 6));
        }
    }
}

#[test]
fn width_limited_admission_keeps_resident_motion_within_the_band_budget() {
    let area = Rect::new(0, 0, 80, 47);
    let growth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(6);
    grid.set_layout(area, growth);
    let mut demands = TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        (1..=19).map(|id| TileDemand { id, rows: 1 }).collect(),
    };
    grid.sync(&demands, growth);
    grid.settle_for_test();
    demands.groups[0].rows = 12;
    grid.sync(&demands, growth);
    let progress = PROGRESS_SCALE / 2;
    grid.measure_row_steps(
        area,
        growth,
        progress,
        |_, content| content.height,
        |_, content| content.height,
    );
    let drawing = grid.drawing_at_with_height_snap(area, growth, progress, &HeightSnap::Off);
    assert_ne!(drawing.pieces, [] as [TilePiece<u32>; 0]);
    for piece in &drawing.pieces {
        let frame = piece.placement.frame.clip();
        assert!(
            frame.height >= 6,
            "a resident must keep its framed floor: {frame:?}"
        );
        assert!(
            frame.top() >= area.top() && frame.bottom() <= area.bottom(),
            "a resident must stay within its band: {frame:?}"
        );
    }
    assert!(
        grid.slots.len() + TABLE_CELL <= 18,
        "two 47-row columns admit at most nine six-row residents each"
    );
    assert_eq!(grid.demands.groups.len(), 19);
}

#[test]
fn no_resident_piece_is_under_the_floor_in_flight() {
    let floor = MIN_TILE_HEIGHT.saturating_mul(2);
    let growth = add_new(4);
    let area = Rect::new(0, 0, TEST_WIDTH, shared_run(4, floor));
    let moving = |from: Vec<Slot<u32>>, to: Vec<Slot<u32>>| {
        let mut grid = TileGrid::new();
        grid.set_min_tile_height(floor);
        grid.set_layout(area, growth);
        grid.held = even(from.len() + TABLE_CELL);
        grid.slots = from;
        grid.queue(vec![to]);
        grid
    };
    let opening = moving(
        vec![Slot::Group(1), Slot::Group(2)],
        vec![Slot::Group(1), Slot::Group(99), Slot::Group(2)],
    );
    let closing = moving(
        vec![Slot::Group(1), Slot::Group(99), Slot::Group(2)],
        vec![Slot::Group(1), Slot::Group(2)],
    );

    for (label, grid) in [("opening", opening), ("closing", closing)] {
        for raw in motion_steps() {
            let drawing = grid.drawing_at(area, growth, raw);
            for resident in [
                TileContent::Summary,
                TileContent::Group(1),
                TileContent::Group(2),
            ] {
                let pieces = drawing
                    .pieces
                    .iter()
                    .filter(|piece| piece.placement.content == resident)
                    .collect::<Vec<_>>();
                assert_eq!(
                    pieces.len(),
                    1,
                    "{label} at {raw} draws one resident {resident:?}"
                );
                let rect = pieces[0].placement.frame.clip();
                assert!(
                    rect.height >= floor,
                    "{label} at {raw} draws {resident:?} under the floor: {rect:?}"
                );
            }
        }
    }
}

#[test]
fn a_shrinking_summary_that_widens_keeps_resident_pieces_at_the_floor() {
    let floor = 6;
    let area = Rect::new(0, 0, 80, 21);
    let growth = widening(3);
    let slots = vec![
        Slot::Group(1),
        Slot::Group(2),
        Slot::Group(3),
        Slot::Group(4),
    ];
    let before = HeldCellLayout {
        rows:          row_demands(&[11, 6, 6, 6, 6]),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let mut grid = TileGrid::new();
    grid.set_min_tile_height(floor);
    grid.set_layout(area, growth);
    grid.slots = slots.clone();
    grid.held = HeldCellLayout {
        rows:          row_demands(&[6, 6, 10, 6, 6]),
        focused:       FocusLocation::Departing,
        summary_span:  2,
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

    for raw in motion_steps().chain([PROGRESS_SCALE / 2]) {
        let drawing = grid.drawing_at(area, growth, raw);
        for piece in &drawing.pieces {
            assert!(
                piece.placement.frame.clip().height >= floor,
                "at {raw}, {:?} is under the floor: {:?}",
                piece.placement.content,
                piece.placement.frame.clip()
            );
        }
    }
}

#[test]
fn fits_holds_at_the_height_floor_and_fails_one_row_under() {
    let configured_floor = MIN_TILE_HEIGHT.saturating_mul(2);
    let mut grid = TileGrid::<u32>::new();
    let growth = add_new(3);
    for (requested, floor, cells) in [
        (0, MIN_TILE_HEIGHT, 3),
        (1, MIN_TILE_HEIGHT, 3),
        (MIN_TILE_HEIGHT - 1, MIN_TILE_HEIGHT, 3),
        (MIN_TILE_HEIGHT, MIN_TILE_HEIGHT, 3),
        (configured_floor, configured_floor, 3),
        (u16::MAX, u16::MAX, 1),
    ] {
        grid.set_min_tile_height(requested);
        assert_eq!(grid.settings.min_tile_height.get(), floor);
        let height = shared_run(cells, floor);
        assert!(fits(
            Rect::new(0, 0, TEST_WIDTH, height),
            usize::from(cells),
            growth,
            &grid.settings
        ));
        assert!(!fits(
            Rect::new(0, 0, TEST_WIDTH, height.saturating_sub(1)),
            usize::from(cells),
            growth,
            &grid.settings
        ));
    }
}

#[test]
fn holds_in_counts_the_headed_summary_depth() {
    let growth = redistribute(1);
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.slots = vec![Slot::Empty(0), Slot::Empty(1), Slot::Empty(2)];
    grid.depth = 2;
    // Two columns hold two floor-height cells each, but not the extra summary position.
    let area = Rect::new(0, 0, 80, 5);

    assert!(fits(area, grid.count(), growth, &grid.settings));
    assert!(!grid.holds_in(area, growth));
}

#[test]
fn falling_tree_demand_keeps_the_first_selected_destination() {
    let area = Rect::new(0, 0, 80, 70);
    let growth = TileGrowth {
        widen_summary: true,
        ..redistribute(8)
    };
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(33);
    grid.set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    grid.set_tile_width_floor(TileWidthFloor::CompressSingleColumn);
    let cells = NarrowTreeDemandCells::default();
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    let destination = grid.placements(area, growth);
    cells.prepared.set(FramePreparation::Unprepared);
    cells.measurements.borrow_mut().clear();
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    assert_eq!(
        grid.placements(area, growth),
        destination,
        "the second draw keeps the selected destination"
    );
    cells.measurements.borrow_mut().clear();
    draw_view_frame(&mut grid, &cells, area, growth);
    assert_eq!(
        cells.measurements.borrow().len(),
        2,
        "an unchanged draw measures only the two displays"
    );
}

#[test]
fn narrow_candidate_row_changes_reselect_a_wide_summary() {
    let area = Rect::new(0, 0, 80, 35);
    let growth = TileGrowth {
        widen_summary: true,
        ..redistribute(4)
    };
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(33);
    grid.set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    let cells = ReflowingSummaryCells {
        arrangement: Cell::new(SummaryLineArrangement::Stacked),
    };
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    draw_view_frame(&mut grid, &cells, area, growth);
    let before = grid.placements(area, growth);
    let columns = before
        .iter()
        .filter(|piece| matches!(piece.content, TileContent::Group(_)))
        .map(|piece| piece.frame.rect().x)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        columns.len(),
        2,
        "the draw must probe a narrow two-column candidate"
    );
    let wide = [(TileContent::Summary, SUMMARY_PREFERRED_WIDTH)];
    let before_rows = crate::TileCells::demands(&cells, &wide).summary;
    cells.arrangement.set(SummaryLineArrangement::Inline);
    assert_eq!(
        crate::TileCells::demands(&cells, &wide).summary,
        before_rows,
        "the current wide summary requests the same rows"
    );
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    draw_view_frame(&mut grid, &cells, area, growth);
    let after = grid.placements(area, growth);
    let summary_width = |placements: &[TilePlacement<u32>]| {
        placements
            .iter()
            .find(|piece| piece.content == TileContent::Summary)
            .expect("the summary remains visible")
            .frame
            .inner()
            .width
    };
    assert!(
        summary_width(&after) < summary_width(&before),
        "the narrow candidate now fits and must replace the wide allocation: before={before:?}, after={after:?}"
    );
}

#[test]
fn the_cells_view_keeps_cells_while_the_summary_draws_fewer_rows_beside_them() {
    let area = Rect::new(0, 0, TEST_TILE_WIDTH.saturating_mul(2), 20);
    let growth = TileGrowth::default();
    let cells = SameFrameSummaryCells { alone_rows: 8 };
    let mut grid = TileGrid::<u32>::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    draw_view_frame(&mut grid, &cells, area, growth);

    assert_eq!(grid.display(), GridDisplay::Cells);
}

#[test]
fn the_cells_view_returns_while_the_summary_is_cut_beside_them() {
    let area = Rect::new(0, 0, TEST_WIDTH, 18);
    let growth = TileGrowth::default();
    let cells = ViewProbeCells {
        summary_demand: 11,
        summary_drawn:  6,
        group_demand:   11,
        group_drawn:    6,
    };
    let mut grid = TileGrid::<u32>::new();
    grid.set_view(TileView::Summary);
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    draw_view_frame(&mut grid, &cells, area, growth);
    assert_eq!(grid.display(), GridDisplay::SummaryAlone);

    grid.set_view(TileView::Cells);
    draw_view_frame(&mut grid, &cells, area, growth);

    assert_eq!(grid.display(), GridDisplay::Cells);
}

/// The summary asks for the fewest columns whose interior holds its
/// widest line, and for all of them when none does.
#[test]
fn the_summary_asks_for_the_fewest_columns_that_hold_its_width() {
    let own = frame_inner(share_borders(
        Rect::new(0, 0, TEST_WIDTH / 2, TEST_HEIGHT),
        test_area(),
    ))
    .width;
    assert_eq!(summary_span(test_area(), 2, 0), 1);
    assert_eq!(summary_span(test_area(), 2, own), 1);
    assert_eq!(summary_span(test_area(), 2, own + 1), 2);
    assert_eq!(summary_span(test_area(), 2, u16::MAX), 2);
    assert_eq!(summary_span(test_area(), 1, u16::MAX), 1);
}

/// Widened, the summary keeps the height its own column gives it and
/// the next column's cells divide what is left below it.
#[test]
fn a_widened_summary_keeps_its_height_and_the_next_column_starts_below_it() {
    let settings = TileSettings::default();
    let narrow = Grid::new(test_area(), &even(4), widening(3), &settings);
    let held = HeldCellLayout {
        summary_span: 2,
        ..even(4)
    };
    let wide = Grid::new(test_area(), &held, widening(3), &settings);
    let cell = |grid: &Grid, index: usize| {
        grid.cell(index)
            .expect("a four-cell grid holds cells one to three")
    };
    let was = cell(&narrow, TABLE_CELL);
    let summary = cell(&wide, TABLE_CELL);
    let under = cell(&wide, TABLE_CELL + 1);
    let beside = cell(&wide, TABLE_CELL + 2);
    assert_eq!(wide.span, 2);
    assert_eq!((summary.y, summary.height), (was.y, was.height));
    assert_eq!((summary.x, summary.right()), (0, TEST_WIDTH));
    assert_eq!(
        beside.y, under.y,
        "the next column starts where the summary ends"
    );
    assert_eq!(beside.x, wide.column_rect(1).x);
}

/// A column whose cells would not fit below the summary stops it,
/// the same as the right edge does.
#[test]
fn a_column_with_no_room_below_the_summary_stops_it() {
    let settings = TileSettings::default();
    let mut held = HeldCellLayout {
        summary_span: 2,
        ..even(4)
    };
    assert_eq!(columns(4, widening(3)), vec![2, 2]);
    assert_eq!(
        Grid::new(test_area(), &held, widening(3), &settings).span,
        2
    );
    held.rows[0] = row_demand(TEST_HEIGHT);
    assert_eq!(
        Grid::new(test_area(), &held, widening(3), &settings).span,
        1
    );
}

/// The grid widens the summary only while the setting is on.
#[test]
fn the_summary_widens_only_when_the_setting_is_on() {
    for (growth, span) in [(redistribute(3), 1), (widening(3), 2)] {
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), growth);
        grid.sync(&wide_summary(&[1, 2, 3], u16::MAX), growth);
        grid.settle_for_test();
        assert_eq!(grid.held.summary_span, span, "{growth:?}");
        let width = grid.content_widths(test_area(), growth)[0].1;
        let expected = if span == 1 {
            TEST_WIDTH / 2 - 1
        } else {
            TEST_WIDTH - 2
        };
        assert_eq!(width, expected, "{growth:?}");
    }
}

/// Up from the top of a column the summary covers reaches the
/// summary, and a sideways step from it passes over what it covers.
#[test]
fn the_arrows_treat_a_widened_summary_as_heading_what_it_covers() {
    let growth = widening(3);
    let mut grid = TileGrid::new();
    grid.set_layout(test_area(), growth);
    grid.sync(&wide_summary(&[1, 2, 3], u16::MAX), growth);
    grid.settle_for_test();
    grid.focus_cell(TABLE_CELL + 2);
    grid.apply(TileAction::FocusUp, growth);
    assert_eq!(grid.focus, Focus::Summary);
    grid.apply(TileAction::FocusRight, growth);
    assert_eq!(grid.focus, Focus::Summary, "nothing stands right of it");
    grid.apply(TileAction::FocusDown, growth);
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
    grid.focus_cell(TABLE_CELL + 3);
    grid.apply(TileAction::FocusLeft, growth);
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
}

/// Below the initial rows there is one column, whichever the fill.
#[test]
fn the_first_column_fills_before_a_second_one_opens() {
    for fill in FILLS {
        let growth = TileGrowth {
            initial_rows: 4,
            fill,
            widen_summary: false,
        };
        assert_eq!(walk(4, growth), vec![vec![1], vec![2], vec![3], vec![4]]);
    }
}

/// The cell past the initial rows does not extend the column and
/// does not open a column of one beside it. It rearranges the whole
/// grid into the smallest square that holds the cells.
#[test]
fn the_cell_past_the_initial_rows_rearranges_the_whole_grid() {
    assert_eq!(columns(4, add_new(3)), vec![2, 2]);
    assert_eq!(columns(5, add_new(4)), vec![3, 2]);
    assert_eq!(columns(9, add_new(8)), vec![3, 3, 3]);
}

/// The whole walk at three initial rows: a column growing to three,
/// then the square it rearranges into and fills a column at a time.
#[test]
fn the_columns_fill_the_smallest_square_that_holds_them() {
    assert_eq!(
        walk(9, add_new(3)),
        vec![
            vec![1],
            vec![2],
            vec![3],
            vec![2, 2],
            vec![3, 2],
            vec![3, 3],
            vec![3, 3, 1],
            vec![3, 3, 2],
            vec![3, 3, 3],
        ]
    );
}

/// The same walk under redistribute: the cell that opens the third
/// column takes one from each of the first two, so it does not
/// stand alone.
#[test]
fn redistribute_keeps_the_columns_within_one_cell_of_each_other() {
    assert_eq!(
        walk(9, redistribute(3)),
        vec![
            vec![1],
            vec![2],
            vec![3],
            vec![2, 2],
            vec![3, 2],
            vec![3, 3],
            vec![3, 2, 2],
            vec![3, 3, 2],
            vec![3, 3, 3],
        ]
    );
}

/// A full square rearranges again rather than opening a column
/// beside itself: the tenth cell at three initial rows makes every
/// column four tall instead of standing alone in a fourth column.
#[test]
fn a_full_square_grows_a_row_before_it_grows_a_column() {
    assert_eq!(columns(10, add_new(3)), vec![4, 4, 2]);
    assert_eq!(columns(12, add_new(3)), vec![4, 4, 4]);
    assert_eq!(columns(13, add_new(3)), vec![4, 4, 4, 1]);
    assert_eq!(columns(16, add_new(3)), vec![4, 4, 4, 4]);
    assert_eq!(columns(17, add_new(3)), vec![5, 5, 5, 2]);
}

/// The fill decides how many cells each column holds and never how
/// many columns there are.
#[test]
fn both_fills_open_the_same_number_of_columns() {
    for initial_rows in 1..=6 {
        for count in 1..=60 {
            assert_eq!(
                columns(count, add_new(initial_rows)).len(),
                columns(count, redistribute(initial_rows)).len(),
                "count {count} at {initial_rows} initial rows"
            );
        }
    }
}

/// Under redistribute no column holds more than one cell over any
/// other, and the taller columns stand to the left of the shorter.
#[test]
fn redistributed_columns_differ_by_at_most_one_taller_first() {
    for initial_rows in 1..=6 {
        for count in 1..=60 {
            let widths = columns(count, redistribute(initial_rows));
            let (Some(&first), Some(&last)) = (widths.first(), widths.last()) else {
                continue;
            };
            assert!(
                first - last <= 1,
                "count {count} at {initial_rows} initial rows: {widths:?}"
            );
            assert!(
                widths.windows(2).all(|pair| pair[0] >= pair[1]),
                "count {count} at {initial_rows} initial rows: {widths:?}"
            );
        }
    }
}

#[test]
fn the_fifth_column_opens_once_every_column_stands_five_tall() {
    assert_eq!(columns(21, add_new(4)), vec![5, 5, 5, 5, 1]);
    assert_eq!(columns(25, add_new(4)), vec![5, 5, 5, 5, 5]);
}

#[test]
fn the_square_after_five_grows_the_same_way() {
    assert_eq!(columns(26, add_new(4)), vec![6, 6, 6, 6, 2]);
    assert_eq!(columns(30, add_new(4)), vec![6, 6, 6, 6, 6]);
    assert_eq!(columns(31, add_new(4)), vec![6, 6, 6, 6, 6, 1]);
    assert_eq!(columns(36, add_new(4)), vec![6, 6, 6, 6, 6, 6]);
}

#[test]
fn initial_rows_below_one_is_treated_as_one() {
    for fill in FILLS {
        assert_eq!(
            columns(
                3,
                TileGrowth {
                    initial_rows: 0,
                    fill,
                    widen_summary: false,
                }
            ),
            columns(
                3,
                TileGrowth {
                    initial_rows: 1,
                    fill,
                    widen_summary: false,
                }
            )
        );
    }
}

/// Nothing is asking for the room going spare, so nothing clever
/// happens: the column is divided evenly.
#[test]
fn a_column_where_everything_fits_divides_evenly() {
    assert_eq!(
        column_rows(&[0, 0, 0, 0], ColumnFocus::Outside),
        apportion(&[1, 1, 1, 1], TEST_HEIGHT)
    );
}

#[test]
fn no_piece_gets_a_row_past_its_ask_while_a_column_mate_is_short() {
    let settings = TileSettings::default();
    let wants = [
        settings.demanded_rows(1),
        settings.demanded_rows(100),
        settings.demanded_rows(12),
    ];
    let mut rows: Vec<u16> = shares(
        &wants,
        TEST_HEIGHT,
        ColumnHead::Cell,
        ColumnFocus::Outside,
        settings.min_tile_height.get(),
    )
    .into_iter()
    .map(|size| match size {
        PaneAxisSize::Fixed(rows) => rows,
        PaneAxisSize::Fill(weight) => weight,
    })
    .collect();
    let shared = rows.len().saturating_sub(1);
    for rows in rows.iter_mut().take(shared) {
        *rows = rows.saturating_add(1);
    }

    assert_eq!(rows, vec![3, 25, 14]);
}

#[test]
fn an_unusable_row_moves_when_its_cell_is_furthest_below_demand() {
    let wants = row_demands(&[100, 20, 20]);
    let rows = shares_with_directional_row_usage(
        &wants,
        TEST_HEIGHT,
        ColumnHead::Cell,
        ColumnFocus::Outside,
        MIN_TILE_HEIGHT,
        &mut |index, rows, _| u16::from(index == 0 && rows == 13),
    );

    assert_eq!(
        rows,
        vec![
            PaneAxisSize::Fixed(12),
            PaneAxisSize::Fixed(13),
            PaneAxisSize::Fixed(15),
        ]
    );
}

#[test]
fn a_loaned_summary_yields_separators_to_hidden_process_rows() {
    let wants = row_demands(&[12, 14]);
    let rows = shares_with_directional_row_usage(
        &wants,
        21,
        ColumnHead::Summary(1),
        ColumnFocus::Outside,
        MIN_TILE_HEIGHT,
        &mut |index, rows, purpose| {
            let used = match (index, purpose) {
                (0, RowUsePurpose::RetainContent) => rows.min(9),
                (0, RowUsePurpose::PaintContent) => rows.min(11),
                _ => rows.min(14),
            };
            rows.saturating_sub(used)
        },
    );
    assert_eq!(rows, vec![PaneAxisSize::Fixed(9), PaneAxisSize::Fixed(12)]);
}

#[test]
fn a_donor_whose_row_use_jumps_as_it_shrinks_keeps_its_used_content() {
    let exact = [32, 8, 7, 10];
    let minimum = [5, 5, 5, 6];
    let mut measured = StartupColumnRows::default();
    for settled in [[29, 6, 5, 9], [30, 6, 5, 8], [28, 6, 5, 10]] {
        for protection in [RowUsePurpose::PaintContent, RowUsePurpose::RetainContent] {
            for index in [0, 3] {
                let held = settled[index];
                let used = held - measured.unused(index, held, protection);
                let spare = directional_spare_rows(
                    index,
                    &settled,
                    &exact,
                    &minimum,
                    protection,
                    &mut measured,
                );
                for donated in 1..=spare {
                    let lower = held - donated;
                    assert_eq!(
                        lower - measured.unused(index, lower, protection),
                        used,
                        "tile {index} loses content at {held}->{lower}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_startup_column_with_nonmonotone_row_use_finishes_sharing() {
    for mut settled in [[29, 6, 5, 9], [30, 6, 5, 8], [28, 6, 5, 10]] {
        let mut measured = StartupColumnRows::default();
        serve_directional_shortfalls(&mut settled, &[32, 8, 7, 10], &[5, 5, 5, 6], &mut measured);
        assert_eq!(settled, [25, 7, 7, 10]);
    }
}

#[test]
fn a_summary_separator_uses_a_mates_unpainted_spare_row() {
    let mut shares = [10, 11];
    let mut unused = |index, rows: u16, purpose| {
        let used = match (index, purpose) {
            (0, RowUsePurpose::RetainContent) => rows.min(9),
            (0, RowUsePurpose::PaintContent) => rows.min(11),
            _ => rows.min(10),
        };
        rows.saturating_sub(used)
    };
    let mut measured = ColumnRowUse::new(shares.len(), &mut unused);
    serve_directional_shortfalls(&mut shares, &[11, 14], &[2, 3], &mut measured);
    assert_eq!(shares, [11, 10]);
}

#[test]
fn a_freed_row_stays_when_no_mates_next_step_fits() {
    let mut settled = [6, 5, 5];
    let exact = [12, 12, 12];
    let minimum = [5, 5, 5];
    let mut unused = |index, rows, _| match (index, rows) {
        (0..=2, 6) => 1,
        _ => 0,
    };

    let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
    serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

    assert_eq!(settled, [6, 5, 5]);
}

#[test]
fn an_exact_demand_change_redivides_without_motion() {
    let growth = add_new(3);
    let demands = |small| TileDemands {
        summary:       small,
        summary_width: 0,
        groups:        vec![
            TileDemand { id: 1, rows: 100 },
            TileDemand { id: 2, rows: 12 },
        ],
    };
    let mut grid = TileGrid::new();
    grid.set_layout(test_area(), growth);
    grid.sync(&demands(1), growth);
    grid.settle_for_test();
    let before = Grid::new(test_area(), &grid.held, growth, &grid.settings)
        .cell(TABLE_CELL)
        .expect("the summary is placed")
        .height;

    grid.sync(&demands(2), growth);

    assert!(matches!(grid.motion, GridMotion::Settled));
    let after = Grid::new(test_area(), &grid.held, growth, &grid.settings)
        .cell(TABLE_CELL)
        .expect("the summary is placed")
        .height;
    assert_eq!((before, after), (3, 4));
}

#[test]
fn a_shrinking_cell_updates_its_exact_demand_across_a_step_boundary() {
    let growth = add_new(3);
    let demands = |last| TileDemands {
        summary:       1,
        summary_width: 0,
        groups:        vec![
            TileDemand { id: 1, rows: 100 },
            TileDemand {
                id:   2,
                rows: last,
            },
        ],
    };
    let mut grid = TileGrid::new();
    grid.set_layout(test_area(), growth);
    grid.sync(&demands(4), growth);
    grid.settle_for_test();
    let heights = |grid: &TileGrid<u32>| {
        let resolved = Grid::new(test_area(), &grid.held, growth, &grid.settings);
        (TABLE_CELL..=TABLE_CELL + 2)
            .map(|cell| {
                resolved
                    .cell(cell)
                    .expect("the column holds the cell")
                    .height
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(heights(&grid), vec![3, 33, 6]);
    grid.sync(&demands(3), growth);
    grid.settle_for_test();

    assert_eq!(heights(&grid), vec![3, 34, 5]);
    assert_eq!(
        grid.held.rows[2].stepped_framed(),
        demanded_rows(4),
        "the held layout retains the earlier step"
    );
    assert_eq!(
        grid.held.rows[2].exact_framed(),
        TileSettings::default().demanded_rows(3).exact_framed(),
        "the held layout accepts the current exact demand"
    );
}

/// Demand is quantised, so a cell has to gain a whole
/// [`TILE_DEMAND_STEP`] before it asks for anything more.
#[test]
fn demand_holds_still_until_it_has_a_whole_step_more_to_ask_for() {
    assert_eq!(demanded_rows(1), demanded_rows(TILE_DEMAND_STEP));
    assert!(demanded_rows(TILE_DEMAND_STEP + 1) > demanded_rows(TILE_DEMAND_STEP));
    assert_eq!(
        demanded_rows(0),
        TILE_BORDER_ROWS,
        "an empty cell asks for nothing beyond its border"
    );
}

/// A scan that moves no cell but changes what one is asking for
/// still travels: the column re-divides over the animation rather
/// than snapping to the new split.
#[test]
fn a_column_re_dividing_travels_the_way_a_cell_moving_does() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    assert!(
        matches!(grid.motion, GridMotion::Settled),
        "the arrangement has settled"
    );

    grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));

    let transition = match &grid.motion {
        GridMotion::Moving(transition) => Some(transition),
        GridMotion::Settled => None,
    }
    .expect("the new demand starts a transition");
    assert_eq!(
        transition.from, grid.slots,
        "no cell moved -- the same slots stand at the same numbers"
    );
    assert_ne!(
        transition.held, grid.held,
        "but the column is divided differently at either end of it"
    );
}

/// Divided by demand or evenly, the cells still tile their area
/// exactly.
#[test]
fn a_column_divided_by_demand_covers_its_area_the_way_an_even_one_does() {
    let mut covered = blank_tally();
    let mut interiors = blank_tally();
    let share = usize::from(TEST_HEIGHT / 4);
    let rows = vec![
        demanded_rows(share * 2),
        0,
        demanded_rows(share + 1),
        0,
        demanded_rows(share * 2),
    ];
    let held = HeldCellLayout {
        rows:          row_demands(&rows),
        focused:       FocusLocation::Cell(2 + TABLE_CELL),
        summary_span:  1,
        summary_depth: 1,
    };
    let grid = Grid::new(
        test_area(),
        &held,
        redistribute(4),
        &TileSettings::default(),
    );
    for index in 1..=held.rows.len() {
        let rect = grid.cell(index).expect("cell is in the grid");
        paint(&mut covered, rect);
        paint(&mut interiors, rect.inner(Margin::new(1, 1)));
    }
    assert!(
        covered.iter().all(|hits| *hits >= 1),
        "the division leaves a gap"
    );
    assert!(
        interiors.iter().all(|hits| *hits <= 1),
        "the division draws two cells into the same place"
    );
}

/// A second column starts on the same screen column the first one
/// ends on. That single shared line is what the grid is drawn from.
#[test]
fn a_column_starts_where_the_one_before_it_ends() {
    let grid = Grid::new(
        test_area(),
        &even(5),
        redistribute(4),
        &TileSettings::default(),
    );
    let first = grid.cell(1).expect("cell is in the grid");
    let second = grid.cell(5).expect("cell is in the grid");
    assert_eq!(first.right() - 1, second.left());
}

#[test]
fn placements_are_the_summary_alone_while_the_cells_are_hidden() {
    let growth = redistribute(4);
    let wide_area = Rect::new(0, 0, 200, TEST_HEIGHT);
    // No core-width tile fits, so only the summary can be placed.
    let narrow_area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_layout(wide_area, growth);
    grid.sync(&quiet(&[1, 2, 3, 4, 5, 6]), growth);
    grid.settle_for_test();
    assert_eq!(grid.drawing(wide_area, growth).column_bands.len(), 3);
    assert_eq!(grid.drawing(narrow_area, growth).column_bands, Vec::new());

    let placements = grid.placements(narrow_area, growth);
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].content, TileContent::Summary);
    assert_eq!(placements[0].frame.rect(), narrow_area);
    assert_eq!(placements[0].frame.clip(), narrow_area);
    assert!(placements[0].frame.is_focused());
}

/// Cell 5 leaves the top of column two and arrives at the bottom of
/// column one, so mid-transition it is drawn twice -- once in each.
#[test]
fn a_cell_changing_column_is_drawn_in_both() {
    let area = test_area();
    let growth = redistribute(4);
    let before = (1..=15).collect::<Vec<_>>();
    let after = (1..=16).collect::<Vec<_>>();
    let grid = synced_motion(&before, &after, growth);
    let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
    let pieces = drawing
        .placements()
        .filter(|placement| placement.content == TileContent::Group(4))
        .collect::<Vec<_>>();
    assert_eq!(pieces.len(), 2);
    let leaving = pieces
        .iter()
        .find(|placement| placement.frame.shift() < 0)
        .expect("the piece leaving travels upward");
    let arriving = pieces
        .iter()
        .find(|placement| placement.frame.shift() > 0)
        .expect("the piece arriving is still below");
    assert_eq!(
        (leaving.frame.clip().x, leaving.frame.clip().width),
        (drawing.column_bands[1].x, drawing.column_bands[1].width),
        "the leaving piece stays in its old column"
    );
    assert_eq!(
        (arriving.frame.clip().x, arriving.frame.clip().width),
        (drawing.column_bands[0].x, drawing.column_bands[0].width),
        "the arriving piece stands in its new column"
    );
    assert_eq!(
        i32::from(leaving.frame.rect().bottom()) + leaving.frame.shift(),
        i32::from(leaving.frame.clip().bottom()),
        "the leaving contents share the top-edge divider"
    );
    assert_eq!(
        i32::from(arriving.frame.rect().top()) + arriving.frame.shift(),
        i32::from(arriving.frame.clip().top()),
        "the arriving contents share the bottom-edge divider"
    );
}

/// The pile of lines a closing column used to leave behind. A cell
/// crossing columns was drawn in the columns as they stood before
/// the step, so while the grid re-divided itself around it the two
/// pieces sat across the cells that had already widened past them --
/// their borders standing inside another cell rather than on a
/// boundary. A wrapping piece shares its edges with whatever stays
/// put in the column it is in.
#[test]
fn a_wrapping_cell_shares_its_column_edges_with_the_cells_that_stay() {
    let area = test_area();
    let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
    let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
    assert_eq!(before.widths, vec![3, 3, 1]);
    assert_eq!(after.widths, vec![3, 3], "the last column goes too");
    let half = PROGRESS_SCALE / 2;

    // Cell four leaves the head of column two for the foot of
    // column one, while cell five stays in column two throughout.
    let bands: Vec<Rect> = (0..before.columns.len().max(after.columns.len()))
        .map(|column| column_band(&before, &after, column, half))
        .collect();
    let mut pieces = Vec::new();
    moving_cell(
        &before,
        &after,
        CellTransition::PresentAtBothEnds {
            before_cell: 4,
            after_cell:  3,
        },
        half,
        drawn(),
        &mut pieces,
    );
    moving_cell(
        &before,
        &after,
        CellTransition::PresentAtBothEnds {
            before_cell: 5,
            after_cell:  4,
        },
        half,
        drawn(),
        &mut pieces,
    );
    let placed = place_band_pieces(&before, &after, &bands, pieces, &HeightSnap::Off);

    let column = |placement: &TilePlacement<u32>| {
        let rect = placement.frame.rect();
        (rect.x, rect.width)
    };
    assert_eq!(
        column(&placed[0].placement),
        column(&placed[2].placement),
        "the piece on its way out stands in the column as it is now"
    );
}

/// A cell whose column goes with it is squeezed off the right edge
/// by the columns widening behind it, rather than left standing
/// where its column used to be.
#[test]
fn a_wrapping_cell_leaving_with_its_column_is_squeezed_off_the_edge() {
    let area = test_area();
    let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
    let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
    let half = PROGRESS_SCALE / 2;

    let out = placed_motion(
        &before,
        &after,
        CellTransition::PresentAtBothEnds {
            before_cell: 7,
            after_cell:  6,
        },
        half,
        drawn(),
    );

    let leaving = out[0].frame.rect();
    assert!(
        leaving.width < before.column_rect(2).width,
        "the column it stands in is being taken away"
    );
    assert_eq!(leaving.right(), area.right(), "against the right edge");
    assert_eq!(out[0].frame.clip(), leaving, "and it is cut off there");
}

/// Closing the one cell a column holds takes the column with it, so
/// the cell is squeezed off the right edge by the columns widening
/// behind it -- one line sweeping sideways, not a second one
/// sliding down at the same time.
#[test]
fn a_cell_taking_its_column_with_it_closes_off_the_right_edge() {
    let area = test_area();
    let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
    let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
    assert_eq!(
        before.widths,
        vec![3, 3, 1],
        "cell seven is a column of one"
    );
    let closed = column_band(&before, &after, 2, PROGRESS_SCALE);
    assert_eq!(closed.width, 1, "only its outer line remains");
    assert_eq!(closed.x, area.right() - 1, "on the last drawable column");
    assert_eq!(
        (closed.y, closed.height),
        (area.y, area.height),
        "and travels nowhere vertically"
    );
}

/// Closing the last cell of a column the grid keeps drops it onto
/// that column's floor, which the cell above expands down onto.
#[test]
fn a_cell_leaving_a_column_that_stays_closes_onto_its_floor() {
    let area = test_area();
    let before = Grid::new(area, &even(6), redistribute(4), &TileSettings::default());
    let after = Grid::new(area, &even(5), redistribute(4), &TileSettings::default());
    assert_eq!(before.widths, vec![3, 3], "cell six sits under cell five");
    let bands = column_bands(&before, &after, PROGRESS_SCALE);
    let mut pieces = Vec::new();
    for cell in 1..=5 {
        moving_cell(
            &before,
            &after,
            CellTransition::PresentAtBothEnds {
                before_cell: cell,
                after_cell:  cell,
            },
            PROGRESS_SCALE,
            drawn(),
            &mut pieces,
        );
    }
    moving_cell(
        &before,
        &after,
        CellTransition::Departing { before_cell: 6 },
        PROGRESS_SCALE,
        drawn(),
        &mut pieces,
    );
    let closed = place_band_pieces(&before, &after, &bands, pieces, &HeightSnap::Off)
        .last()
        .map(|piece| piece.placement.frame.clip())
        .unwrap_or_default();
    assert_eq!(closed.height, 1, "only its edge remains");
    assert_eq!(
        closed.y,
        after.column_rect(1).bottom() - 1,
        "against the column floor"
    );
    assert_eq!(
        (closed.x, closed.width),
        (after.column_rect(1).x, after.column_rect(1).width)
    );
}

#[test]
fn every_queued_state_keeps_surviving_cells_in_an_endpoint_column() {
    let growth = redistribute(2);
    let area = Rect {
        height: TEST_HEIGHT.saturating_add(1),
        ..test_area()
    };
    for (before, after, start_depth, final_depth) in [
        (&[1, 2][..], &[1, 99, 2][..], 2, 3),
        (&[1, 99, 2][..], &[1, 2][..], 3, 2),
    ] {
        let demands = |ids: &[u32]| TileDemands {
            summary:       37,
            summary_width: 0,
            groups:        ids.iter().map(|&id| TileDemand { id, rows: 0 }).collect(),
        };
        let mut grid = seeded_grid();
        grid.set_layout(area, growth);
        grid.sync(&demands(before), growth);
        grid.settle_for_test();
        assert_eq!(grid.depth, start_depth);
        let start = grid.slots.clone();

        grid.sync(&demands(after), growth);
        let finish = grid.target();
        assert_eq!(grid.target_depth(), final_depth);
        assert_eq!(
            finish,
            after.iter().copied().map(Slot::Group).collect::<Vec<_>>()
        );
        for (slots, depth) in std::iter::once((&grid.slots, grid.depth))
            .chain(grid.pending.iter().map(|step| (&step.slots, step.depth)))
        {
            assert!(
                queued_cells_keep_endpoint_columns(
                    &QueuedArrangement {
                        slots:         &start,
                        summary_depth: start_depth,
                    },
                    &QueuedArrangement {
                        slots:         &finish,
                        summary_depth: final_depth,
                    },
                    &QueuedArrangement {
                        slots,
                        summary_depth: depth
                    },
                    growth,
                    area,
                    &grid.settings,
                ),
                "{before:?}@{start_depth} -> {after:?}@{final_depth} queued {slots:?}@{depth}"
            );
        }
    }
}

#[test]
fn adding_the_tenth_live_cell_never_draws_a_survivor_in_a_third_column() {
    let area = Rect::new(0, 0, 200, 50);
    let growth = TileGrowth::default();
    let demands = TileDemands {
        summary:       13,
        summary_width: 0,
        groups:        (1..=8).map(|id| TileDemand { id, rows: 13 }).collect(),
    };
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&demands, growth);
    grid.settle_for_test();
    assert_eq!(grid.count(), 9);
    assert_eq!(grid.depth, 1);

    grid.apply(TileAction::Add, growth);
    assert_eq!(grid.target().len() + TABLE_CELL, 10);
    assert_eq!(grid.target_depth(), 1);

    let mut steps = 0;
    while let GridMotion::Moving(transition) = &grid.motion {
        steps += 1;
        let before = Grid::new(area, &transition.held, growth, &grid.settings);
        let after_held = grid.drawn_held();
        let after = Grid::new(area, &after_held, growth, &grid.settings);
        let survivors = transition
            .from
            .iter()
            .filter(|slot| grid.slots.contains(slot))
            .filter_map(|slot| match slot {
                Slot::Group(id) => Some((*id, slot)),
                Slot::Empty(_) => None,
            })
            .collect::<Vec<_>>();
        for raw in motion_steps() {
            let drawing = grid.drawing_at(area, growth, raw);
            for &(id, slot) in &survivors {
                let Some(before_cell) = cell_of(&transition.from, slot) else {
                    continue;
                };
                let Some(after_cell) = cell_of(&grid.slots, slot) else {
                    continue;
                };
                let endpoint_columns = [before.column_of(before_cell), after.column_of(after_cell)];
                for placement in drawing
                    .placements()
                    .filter(|placement| placement.content == TileContent::Group(id))
                {
                    let clip = placement.frame.clip();
                    assert!(
                        endpoint_columns.into_iter().flatten().any(|column| {
                            drawing
                                .column_bands
                                .get(column)
                                .is_some_and(|band| clip.x == band.x && clip.width == band.width)
                        }),
                        "step {steps} at raw {raw} draws {id} in {clip:?}, outside endpoint columns {endpoint_columns:?}"
                    );
                }
            }
        }
        grid.advance();
    }
    assert!(steps > 0, "adding the tenth cell queues motion");
}

#[test]
fn a_key_that_changes_slots_and_depth_queues_one_step() {
    let (mut grid, _, growth) = depth_changing_key_grid();
    for (action, final_depth) in [(TileAction::Add, 1), (TileAction::Remove, 2)] {
        assert!(matches!(grid.motion, GridMotion::Settled));
        grid.apply(action, growth);
        assert!(matches!(grid.motion, GridMotion::Moving(_)));
        assert_eq!(grid.depth, final_depth);
        assert!(grid.pending.is_empty());
        grid.advance();
    }
}

#[test]
fn a_key_whose_new_widths_change_demands_still_moves_once() {
    let area = Rect::new(0, 0, TEST_TILE_WIDTH.saturating_mul(3), TEST_HEIGHT);
    let growth = TileGrowth::default();
    let cells = WidthDependentSummaryCells;
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(6);
    draw_view_frame(&mut grid, &cells, area, growth);
    grid.settle_for_test();
    draw_view_frame(&mut grid, &cells, area, growth);
    assert_eq!(grid.count(), 6);
    assert_eq!(grid.depth, 1);

    assert_width_changing_key_moves_once(&mut grid, &cells, area, growth, TileAction::Add, 2);
    assert_width_changing_key_moves_once(&mut grid, &cells, area, growth, TileAction::Remove, 1);
}

#[test]
fn a_previous_empty_height_below_the_destination_floor_keeps_ordered_bounds() {
    let (mut grid, area, growth) =
        exact_slot_height_motion(&[9, 3], &[6, 6], vec![Slot::Empty(0)], vec![Slot::Empty(0)]);
    let previous = grid.drawing_at_with_height_snap(area, growth, 0, &HeightSnap::Off);
    let heights = previous
        .placements()
        .map(|placement| placement.frame.clip().height)
        .collect::<Vec<_>>();
    assert_eq!(heights, [9, 3]);
    grid.motion_heights = MotionHeightPath {
        area,
        growth,
        progress: 0,
        piece_heights: heights,
    };
    grid.set_min_tile_height(6);
    grid.demands = TileDemands {
        summary:       100,
        summary_width: 0,
        groups:        Vec::new(),
    };

    grid.measure_row_steps(
        area,
        growth,
        1,
        |content, area| match content {
            TileContent::Empty(_) => 0,
            _ => area.height,
        },
        |content, area| match content {
            TileContent::Empty(_) => 0,
            _ => area.height,
        },
    );

    let destination = grid.resolved_grid(area, &grid.drawn_held(), growth);
    assert_eq!(destination.rules.min_tile_height.get(), 6);
    assert_eq!(destination.resolved.panes.len(), 2);
    for pane in &destination.resolved.panes {
        assert!(
            (destination.rules.min_tile_height.get()..=area.height).contains(&pane.area.height)
        );
    }
    assert_eq!(destination.resolved.panes[1].area.height, 6);
}

#[test]
fn a_departure_uses_recorded_heights_when_its_rows_drop_to_zero() {
    let area = test_area();
    let growth = redistribute(4);
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.sync(&tall_demands(&[1, 2, 3]), growth);
    grid.settle_for_test();
    let recorded = recorded_settled_heights(&mut grid, area, growth);
    grid.sync(&tall_demands(&[1, 3]), growth);

    let remaining = [
        TileContent::Summary,
        TileContent::Group(1),
        TileContent::Group(3),
    ];
    let mut paths = vec![Vec::new(); remaining.len()];
    for raw in motion_steps() {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| match content {
                TileContent::Group(2) | TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
            |content, area| match content {
                TileContent::Group(2) | TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
        );
        let drawing = grid.drawing_at(area, growth, raw);
        if raw == 0 {
            for (content, height) in &recorded {
                assert_eq!(piece_height(&drawing, content), Some(*height));
            }
        }
        for (content, heights) in remaining.iter().zip(&mut paths) {
            heights.push(
                piece_height(&drawing, content).expect("each remaining piece stays in the drawing"),
            );
        }
    }

    for (content, heights) in remaining.iter().zip(paths) {
        let start = recorded
            .iter()
            .find_map(|(recorded, height)| (recorded == content).then_some(*height))
            .expect("the settled frame contains every remaining piece");
        let end = heights.last().copied().unwrap_or(start);
        let one_way = if start <= end {
            heights.iter().all(|height| *height >= start)
                && heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.iter().all(|height| *height <= start)
                && heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(
            one_way,
            "{content:?} reverses from {start} through {heights:?}"
        );
    }
}

#[test]
fn a_resize_during_motion_draws_every_column_inside_the_new_area() {
    let old_area = Rect::new(0, 0, 80, 30);
    let new_area = Rect::new(10, 5, 60, 22);
    let growth = redistribute(1);
    let slots = (1..=3).map(Slot::Group).collect::<Vec<_>>();
    let source_held = HeldCellLayout {
        rows:          row_demands(&[8, 8, 8, 8]),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    let destination_held = HeldCellLayout {
        rows: row_demands(&[10, 6, 10, 6]),
        ..source_held
    };
    let source = Grid::new(old_area, &source_held, growth, &TileSettings::default());
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(old_area, growth);
    grid.slots = slots.clone();
    grid.held = destination_held;
    grid.motion = GridMotion::Moving(Transition {
        from:         slots,
        held:         source_held,
        source:       Box::new(source),
        started:      Instant::now(),
        millis:       1,
        depth_status: DestinationDepthStatus::Resolved,
    });
    let _ = filled_height_drawing(
        &mut grid,
        old_area,
        growth,
        PROGRESS_SCALE.saturating_div(4),
    );

    grid.set_layout(new_area, growth);
    for raw in PROGRESS_SCALE.saturating_div(4)..=PROGRESS_SCALE {
        let drawing = filled_height_drawing(&mut grid, new_area, growth, raw);

        assert_eq!(drawing.column_bands.len(), 2);
        for band in &drawing.column_bands {
            assert_eq!(
                (band.top(), band.bottom()),
                (new_area.top(), new_area.bottom()),
                "column outer rows at {raw}"
            );
            assert!(band.left() >= new_area.left(), "column left at {raw}");
            assert!(band.right() <= new_area.right(), "column right at {raw}");
        }
        for placement in drawing.placements() {
            let clip = placement.frame.clip();
            assert!(clip.left() >= new_area.left(), "piece left at {raw}");
            assert!(clip.right() <= new_area.right(), "piece right at {raw}");
            assert!(clip.top() >= new_area.top(), "piece top at {raw}");
            assert!(clip.bottom() <= new_area.bottom(), "piece bottom at {raw}");
        }
    }
}

#[test]
fn a_growing_resident_uses_the_current_frames_whole_row_count() {
    let content = TileContent::<u32>::Summary;
    let rect = Rect::new(0, 0, 40, 10);
    let area = draw::content_area(frame_inner(rect));
    let mut usage = Vec::new();
    measure_frame_row_usage(&mut usage, &content, area, &mut |_, area| {
        if area.height == u16::MAX { 9 } else { 6 }
    });
    let piece = BandPiece {
        column:   0,
        motion:   BandPieceMotion::Resident {
            before: Rect::new(0, 0, 40, 9),
            after:  Rect::new(0, 0, 40, 12),
        },
        progress: PROGRESS_SCALE / 2,
        drawn:    CellAppearance {
            identity: match &content {
                TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                _ => CellIdentity::Summary,
            },
            content,
            focused: false,
        },
    };

    assert!(
        !HeightSnap::Frame {
            row_usage: &usage,
            path:      PieceHeightPath::AdvanceFrom(&[]),
        }
        .accepts(&piece, rect)
    );
}

#[test]
fn a_widening_resident_uses_its_current_widths_whole_row_count() {
    let content = TileContent::<u32>::Summary;
    let rect = Rect::new(0, 0, 40, 10);
    let area = draw::content_area(frame_inner(rect));
    let mut usage = Vec::new();
    measure_frame_row_usage(&mut usage, &content, area, &mut |_, area| {
        if area.height == u16::MAX { 9 } else { 6 }
    });
    let piece = BandPiece {
        column:   0,
        motion:   BandPieceMotion::Resident {
            before: Rect::new(0, 0, 30, 10),
            after:  Rect::new(0, 0, 50, 10),
        },
        progress: PROGRESS_SCALE / 2,
        drawn:    CellAppearance {
            identity: match &content {
                TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                _ => CellIdentity::Summary,
            },
            content,
            focused: false,
        },
    };

    assert!(
        !HeightSnap::Frame {
            row_usage: &usage,
            path:      PieceHeightPath::AdvanceFrom(&[]),
        }
        .accepts(&piece, rect)
    );
}

#[test]
fn a_whole_moving_piece_gives_spare_rows_to_a_cutting_mate() {
    let compact_height = MIN_TILE_HEIGHT.saturating_mul(2);
    let expanded_height = compact_height.saturating_add(4);
    let (grid, area, growth) = exact_height_motion(
        &[compact_height, expanded_height],
        &[expanded_height, compact_height],
    );
    let whole_rows = draw::content_area(frame_inner(Rect {
        height: compact_height,
        ..area
    }))
    .height;

    for raw in 1..PROGRESS_SCALE {
        let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
        let mut row_usage = Vec::new();
        for piece in &unsnapped.pieces {
            for height in compact_height..=expanded_height {
                let content_area = draw::content_area(frame_inner(Rect {
                    height,
                    ..piece.placement.frame.clip()
                }));
                measure_frame_row_use(
                    &mut row_usage,
                    &piece.placement.content,
                    content_area,
                    &mut |content, area| match content {
                        TileContent::Summary => whole_rows.min(area.height),
                        TileContent::Group(_) | TileContent::Empty(_) => area.height,
                    },
                    &mut |content, area| match content {
                        TileContent::Summary => whole_rows.min(area.height),
                        TileContent::Group(_) | TileContent::Empty(_) => area.height,
                    },
                );
            }
        }
        let height_snap = HeightSnap::Frame {
            row_usage: &row_usage,
            path:      PieceHeightPath::AdvanceFrom(&[]),
        };
        let drawing = grid.drawing_at_with_height_snap(area, growth, raw, &height_snap);
        let summary = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("the summary remains in the band");
        let summary_area = draw::content_area(frame_inner(summary.frame.clip()));
        assert_eq!(
            summary_area.height, whole_rows,
            "the summary keeps a spare row while its mate cuts at {raw}"
        );
    }
}

#[test]
fn a_summary_shrink_starts_filled_and_never_reverses() {
    let (mut grid, area, growth) = exact_height_motion(&[10, 8], &[6, 12]);
    let mut heights = Vec::new();
    for raw in 0..=PROGRESS_SCALE {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| match content {
                TileContent::Summary if area.height == u16::MAX => 15,
                TileContent::Summary if [5, 7].contains(&area.height) => {
                    area.height.saturating_sub(1)
                },
                TileContent::Summary | TileContent::Group(_) => area.height,
                TileContent::Empty(_) => 0,
            },
            |content, area| match content {
                TileContent::Summary if [5, 7].contains(&area.height) => {
                    area.height.saturating_sub(1)
                },
                TileContent::Summary | TileContent::Group(_) => area.height,
                TileContent::Empty(_) => 0,
            },
        );
        let drawing = grid.drawing_at(area, growth, raw);
        let summary = drawing
            .pieces
            .iter()
            .find(|piece| piece.placement.content == TileContent::Summary)
            .expect("the summary remains in the shrinking band");
        let content_height = draw::content_area(frame_inner(summary.placement.frame.clip())).height;
        let rows = if [5, 7].contains(&content_height) {
            content_height.saturating_sub(1)
        } else {
            content_height
        };
        assert_eq!(
            rows, content_height,
            "summary leaves a row at progress {raw}"
        );
        heights.push(summary.placement.frame.clip().height);
    }
    assert_eq!(heights.first().copied(), Some(9));
    assert!(
        heights.windows(2).all(|pair| pair[0] >= pair[1]),
        "the shrinking summary reverses through {heights:?}"
    );
}

#[test]
fn a_content_change_keeps_residents_at_their_last_filled_heights() {
    let (mut grid, area, growth) = exact_height_motion(&[19, 27], &[27, 19]);
    let mut last_heights = vec![19, 27];
    for raw in 0..=PROGRESS_SCALE {
        let valid_heights = (PROGRESS_SCALE / 2..PROGRESS_SCALE)
            .contains(&raw)
            .then(|| last_heights.clone());
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| {
                if area.height == u16::MAX {
                    return TEST_HEIGHT;
                }
                let Some(valid_heights) = &valid_heights else {
                    return area.height;
                };
                let index = match content {
                    TileContent::Summary => 0,
                    TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
                    TileContent::Empty(_) => return 0,
                };
                let framed = area
                    .height
                    .saturating_add(TILE_BORDER_ROWS)
                    .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                if valid_heights.get(index).copied() == Some(framed) {
                    area.height
                } else {
                    area.height.saturating_sub(1)
                }
            },
            |_, area| area.height,
        );
        let drawing = grid.drawing_at(area, growth, raw);
        let heights = drawing
            .placements()
            .map(|placement| placement.frame.clip().height)
            .collect::<Vec<_>>();
        if let Some(valid_heights) = valid_heights {
            assert_eq!(
                heights, valid_heights,
                "a resident leaves its last filled height at {raw}"
            );
        }
        last_heights = heights;
    }
}

#[test]
fn a_column_still_tiles_its_band_while_pieces_snap() {
    let (mut grid, area, growth) = stepped_height_motion();
    let mut calls = 0;
    for raw in 0..=PROGRESS_SCALE {
        let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
        for &band in &drawing.column_bands {
            let pieces = drawing
                .placements()
                .map(|placement| placement.frame.clip())
                .filter(|piece| piece.x == band.x && piece.width == band.width)
                .collect::<Vec<_>>();
            assert_eq!(pieces.first().map(|piece| piece.top()), Some(band.top()));
            assert_eq!(
                pieces.last().map(|piece| piece.bottom()),
                Some(band.bottom())
            );
            assert!(
                pieces
                    .windows(2)
                    .all(|pair| pair[0].bottom().saturating_sub(1) == pair[1].top())
            );
        }
    }
}

#[test]
fn arriving_and_leaving_pieces_snap_without_a_floor() {
    let (mut arriving, area, growth) = stepped_height_motion();
    let mut calls = 0;
    let mut arrival_below_floor = false;
    for raw in arriving.distinct_motion_steps_for_test(area, growth) {
        let drawing = stepped_height_drawing(&mut arriving, area, growth, raw, &mut calls);
        let rect = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Group(2))
            .map(|placement| placement.frame.clip());
        assert!(rect.is_some(), "the arrival has one piece at {raw}");
        let rect = rect.unwrap_or_default();
        arrival_below_floor |= rect.height < MIN_TILE_HEIGHT;
        assert!(
            stepped_height_fills(&TileContent::Group(2), rect),
            "arrival at {raw}: {rect:?}"
        );
    }
    assert!(
        arrival_below_floor,
        "the arrival may collapse under the floor"
    );

    let mut leaving = TileGrid::new();
    leaving.set_view(TileView::Cells);
    leaving.set_layout(area, growth);
    leaving.sync(&tall_demands(&[1, 2]), growth);
    leaving.settle_for_test();
    leaving.collapse_queued_changes_for_test();
    leaving.sync(&tall_demands(&[1]), growth);
    let mut departure_below_floor = false;
    for raw in leaving.distinct_motion_steps_for_test(area, growth) {
        let drawing = stepped_height_drawing(&mut leaving, area, growth, raw, &mut calls);
        let rect = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Group(2))
            .map(|placement| placement.frame.clip())
            .expect("the departure has one piece");
        departure_below_floor |= rect.height < MIN_TILE_HEIGHT;
        assert!(
            stepped_height_fills(&TileContent::Group(2), rect),
            "departure at {raw}: {rect:?}"
        );
    }
    assert!(
        departure_below_floor,
        "the departure may collapse under the floor"
    );

    let before = [1, 2, 3, 4, 5, 6];
    let after = [1, 2, 3, 4, 6];
    let closing_growth = redistribute(3);
    let mut closing = synced_motion(&before, &after, closing_growth);
    let start = closing.drawing_at(area, closing_growth, 0);
    let column = start.column_bands.len().saturating_sub(1);
    let start_band = start.column_bands[column];
    let starting_rows = start
        .placements()
        .filter(|placement| {
            let clip = placement.frame.clip();
            clip.x == start_band.x && clip.width == start_band.width
        })
        .map(|placement| {
            let clip = placement.frame.clip();
            (clip.y, clip.height)
        })
        .collect::<Vec<_>>();
    let middle = stepped_height_drawing(
        &mut closing,
        area,
        closing_growth,
        PROGRESS_SCALE / 2,
        &mut calls,
    );
    let middle_band = middle.column_bands[column];
    let middle_rows = middle
        .placements()
        .filter(|placement| {
            let clip = placement.frame.clip();
            clip.x == middle_band.x && clip.width == middle_band.width
        })
        .map(|placement| {
            let clip = placement.frame.clip();
            (clip.y, clip.height)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        middle_rows, starting_rows,
        "the closing column keeps its rows"
    );
}

#[test]
fn a_large_cell_column_measures_each_allocation_once() {
    for width in LARGE_CONTENT_TEST_WIDTHS {
        for rows in LARGE_CONTENT_TEST_ROWS {
            let area = Rect::new(0, 0, width, LARGE_CONTENT_TEST_HEIGHT);
            let held = HeldCellLayout {
                rows:          row_demands(&[rows; LARGE_CONTENT_TEST_CELLS]),
                focused:       FocusLocation::Departing,
                summary_span:  1,
                summary_depth: 1,
            };
            let mut queries = Vec::new();
            let grid = Grid::with_row_use(
                area,
                &held,
                add_new(LARGE_CONTENT_TEST_INITIAL_ROWS),
                &TileSettings::default(),
                &mut |index, area, purpose| {
                    let retained = matches!(purpose, RowUsePurpose::RetainContent);
                    let key = (index, area.width, area.height, retained);
                    assert!(
                        !queries.contains(&key),
                        "repeated column allocation: {key:?}"
                    );
                    queries.push(key);
                    if index == 0 {
                        area.height.min(rows) / LARGE_CONTENT_TEST_ROW_STEP
                            * LARGE_CONTENT_TEST_ROW_STEP
                    } else {
                        area.height.min(1)
                    }
                },
            );
            assert_eq!(grid.resolved.panes.len(), held.rows.len());
            assert!(
                (0..held.rows.len()).all(|index| queries.iter().any(|(known, ..)| *known == index))
            );
        }
    }
}

#[test]
fn changed_retained_rows_update_the_next_settled_transfer() {
    let area = Rect::new(0, 0, TEST_WIDTH, 29);
    let growth = add_new(5);
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_layout(area, growth);
    grid.slots = (1..=4).map(Slot::Group).collect();
    grid.held = HeldCellLayout {
        rows:          row_demands(&[17, 11, 15, 15, 12]),
        focused:       FocusLocation::Departing,
        summary_span:  1,
        summary_depth: 1,
    };
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |_, area| area.height,
        |_, area| area.height,
    );
    let before = grid
        .resolved_grid(area, &grid.drawn_held(), growth)
        .cell(TABLE_CELL)
        .expect("the first pass places the summary")
        .height;

    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |_, area| area.height,
        |content, area| match content {
            TileContent::Summary | TileContent::Group(2 | 3) if area.height == 4 => {
                area.height.saturating_sub(1)
            },
            TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
        },
    );
    let after = grid
        .resolved_grid(area, &grid.drawn_held(), growth)
        .cell(TABLE_CELL)
        .expect("the refreshed pass places the summary")
        .height;

    assert!(after > before, "the summary receives a newly freed row");
    assert!(!grid.tick());
}

#[test]
fn column_bands_tile_the_area_through_a_transition() {
    let growth = redistribute(4);
    let opening = opening_column();
    let mut closing = opening_column();
    closing.settle_for_test();
    closing.remove();

    for grid in [&opening, &closing] {
        for raw in motion_steps() {
            let bands = grid.drawing_at(test_area(), growth, raw).column_bands;
            assert_eq!(
                bands.first().map(|band| band.left()),
                Some(test_area().left())
            );
            assert_eq!(
                bands.last().map(|band| band.right() - 1),
                Some(test_area().right() - 1)
            );
            for pair in bands.windows(2) {
                assert_eq!(pair[0].right() - 1, pair[1].left());
            }
        }
    }
}

#[test]
fn pieces_in_a_column_share_their_separators_through_a_transition() {
    let growth = redistribute(4);
    let opening = opening_column();
    assert_shared_piece_rows(&opening, growth);

    let mut closing = opening_column();
    closing.settle_for_test();
    closing.remove();
    assert_shared_piece_rows(&closing, growth);

    let mut growing = seeded_grid();
    growing.sync(&quiet(&[1]), growth);
    growing.settle_for_test();
    growing.sync(&busy(&[(1, usize::from(TEST_HEIGHT))]), growth);
    assert_shared_piece_rows(&growing, growth);

    let mut crossing = seeded_grid();
    let before: Vec<u32> = (1..=15).collect();
    let after: Vec<u32> = (1..=16).collect();
    crossing.sync(&quiet(&before), growth);
    crossing.settle_for_test();
    crossing.sync(&quiet(&after), growth);
    assert_shared_piece_rows(&crossing, growth);
}

#[test]
fn a_motion_frame_applies_unused_rows_to_its_own_arrangement() {
    let area = Rect::new(0, 0, TEST_WIDTH, 40);
    let growth = redistribute(4);
    let demands = |groups: &[u32]| TileDemands {
        summary:       100,
        summary_width: 0,
        groups:        groups
            .iter()
            .map(|&id| TileDemand { id, rows: 100 })
            .collect(),
    };
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&demands(&[1, 2, 3]), growth);
    grid.settle_for_test();
    grid.sync(&demands(&[1, 3]), growth);
    let GridMotion::Moving(transition) = &grid.motion else {
        assert!(
            matches!(&grid.motion, GridMotion::Moving(_)),
            "the removed cell starts one motion"
        );
        return;
    };
    let source = Grid::new(area, &transition.held, growth, &grid.settings)
        .cell(TABLE_CELL)
        .expect("the source places the summary");

    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE / 2,
        |content, area| match content {
            TileContent::Group(3) => area.height.saturating_sub(1),
            TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
        },
        |content, area| match content {
            TileContent::Group(3) => area.height.saturating_sub(1),
            TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
        },
    );

    let destination = grid
        .resolved_grid(area, &grid.drawn_held(), growth)
        .cell(TABLE_CELL)
        .expect("the destination places the summary");
    let summary_at = |raw| {
        grid.drawing_at(area, growth, raw)
            .pieces
            .into_iter()
            .find(|piece| piece.placement.content == TileContent::Summary)
            .map(|piece| piece.placement.frame.rect())
            .expect("the frame places the summary")
    };

    assert_eq!(summary_at(0), source);
    assert_eq!(summary_at(PROGRESS_SCALE), destination);
}

#[test]
fn a_cell_claimed_during_an_opening_stays_in_its_transition_column() {
    let growth = redistribute(4);
    let mut grid = opening_column();
    let opening_from = match &grid.motion {
        GridMotion::Moving(transition) => Some(transition.from.clone()),
        GridMotion::Settled => None,
    };
    assert!(opening_from.is_some(), "the column opening is in flight");
    let opening_from = opening_from.unwrap_or_default();

    grid.sync(&quiet(&[1, 2, 3, 99]), growth);
    let still_from = match &grid.motion {
        GridMotion::Moving(transition) => Some(transition.from.clone()),
        GridMotion::Settled => None,
    };
    assert!(
        still_from.is_some(),
        "claiming the cell preserves the opening"
    );
    let still_from = still_from.unwrap_or_default();
    assert_eq!(still_from, opening_from);
    for raw in motion_steps() {
        assert!(
            grid.drawing_at(test_area(), growth, raw)
                .placements()
                .all(|placement| placement.content != TileContent::Group(99)),
            "the claim waits for the opening at {raw}"
        );
    }

    grid.advance();
    for raw in motion_steps() {
        let claimed = grid
            .drawing_at(test_area(), growth, raw)
            .placements()
            .filter(|placement| placement.content == TileContent::Group(99))
            .count();
        assert_eq!(claimed, 1, "one claimed piece is drawn at {raw}");
    }
}

/// A cell is placed where there is room for it, which need not be
/// where it belongs. Whatever order the cells were placed in, they
/// end up standing in the order the demands name them -- otherwise
/// a command that opened its cell early sits above one that started
/// before it, for as long as both live.
#[test]
fn the_cells_stand_in_the_order_they_are_demanded_in() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[8]), redistribute(4));
    grid.settle_for_test();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(7), TileContent::Group(8),]
    );
}

/// Every piece leaves with a closing column at its starting rows,
/// including when a wide summary has shortened the column.
#[test]
fn no_piece_in_a_closing_column_changes_height() {
    let area = test_area();
    let before = [1, 2, 3, 4, 5, 6];
    let after = [1, 2, 3, 4, 6];
    let ordinary_growth = redistribute(3);
    let ordinary = synced_motion(&before, &after, ordinary_growth);

    let covered_growth = widening(3);
    let mut covered = seeded_grid();
    covered.set_layout(area, covered_growth);
    covered.sync(&wide_summary(&before, u16::MAX), covered_growth);
    covered.settle_for_test();
    covered.sync(&wide_summary(&after, u16::MAX), covered_growth);

    for (label, grid, growth, shortened) in [
        ("full-height", ordinary, ordinary_growth, false),
        ("under the summary", covered, covered_growth, true),
    ] {
        let start = grid.drawing_at(area, growth, 0);
        assert_eq!(start.column_bands.len(), 3, "{label} column count");
        let column = start.column_bands.len() - 1;
        let start_band = start.column_bands[column];
        assert_eq!(
            start_band.top() > area.top(),
            shortened,
            "{label} starting top"
        );
        let starting_rows = start
            .placements()
            .filter(|placement| {
                let clip = placement.frame.clip();
                clip.x == start_band.x && clip.width == start_band.width
            })
            .map(|placement| {
                let rect = placement.frame.rect();
                let clip = placement.frame.clip();
                (
                    placement.content.clone(),
                    rect.y,
                    rect.height,
                    clip.y,
                    clip.height,
                    placement.frame.shift(),
                )
            })
            .collect::<Vec<_>>();
        assert!(starting_rows.len() > 1, "{label} has stacked pieces");

        for raw in motion_steps() {
            let drawing = grid.drawing_at(area, growth, raw);
            let band = drawing.column_bands[column];
            let rows = drawing
                .placements()
                .filter(|placement| {
                    let clip = placement.frame.clip();
                    clip.x == band.x && clip.width == band.width
                })
                .map(|placement| {
                    let rect = placement.frame.rect();
                    let clip = placement.frame.clip();
                    (
                        placement.content.clone(),
                        rect.y,
                        rect.height,
                        clip.y,
                        clip.height,
                        placement.frame.shift(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(rows, starting_rows, "{label} at raw {raw}");
            assert_eq!(
                (band.top(), band.bottom()),
                (start_band.top(), start_band.bottom()),
                "{label} band at raw {raw}"
            );
        }
    }
}

/// The same cell in a column the grid keeps does slide off its
/// edge, which is the snake every other close is drawn as.
#[test]
fn a_cell_leaving_a_column_that_stays_slides_off_its_edge() {
    let area = test_area();
    let growth = redistribute(4);
    let before = (1..=15).collect::<Vec<_>>();
    let after = (1..=16).collect::<Vec<_>>();
    let grid = synced_motion(&before, &after, growth);
    let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
    let leaving = drawing
        .placements()
        .filter(|placement| placement.content == TileContent::Group(4))
        .find(|placement| placement.frame.shift() < 0)
        .expect("the piece leaving travels upward");

    assert_eq!(
        i32::from(leaving.frame.rect().bottom()) + leaving.frame.shift(),
        i32::from(leaving.frame.clip().bottom()),
        "its bottom border stays on the divider it leaves through"
    );
}

#[test]
fn a_middle_departure_starts_on_its_before_rows_between_its_neighbours() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();
    grid.sync(&quiet(&[7, 9]), redistribute(4));

    let drawing = grid.drawing_at(test_area(), redistribute(4), 0);
    let placement = |content| {
        drawing
            .placements()
            .find(|placement| placement.content == content)
            .expect("the transition still draws this command")
    };
    let above = placement(TileContent::Group(7));
    let survivor = placement(TileContent::Group(9));
    let departing = placement(TileContent::Group(8));
    let band = drawing
        .column_bands
        .first()
        .copied()
        .expect("the transition has a column");

    assert_eq!(
        departing.frame.clip().top(),
        above.frame.clip().bottom() - 1,
        "the departure shares its upper separator"
    );
    assert_eq!(
        departing.frame.clip().bottom() - 1,
        survivor.frame.clip().top(),
        "the departure shares its lower separator"
    );
    assert_eq!(
        survivor.frame.clip().bottom(),
        band.bottom(),
        "the last survivor reaches the column floor"
    );
    assert_eq!(
        departing.frame.clip(),
        Grid::new(
            test_area(),
            &even(4),
            redistribute(4),
            &TileSettings::default(),
        )
        .cell(3)
        .expect("the before grid has the departing cell"),
        "the departing piece begins on its settled before rect"
    );
}

struct ViewProbeCells {
    summary_demand: usize,
    summary_drawn:  u16,
    group_demand:   usize,
    group_drawn:    u16,
}

impl crate::TileCells<u32> for ViewProbeCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       self.summary_demand,
            summary_width: 0,
            groups:        vec![TileDemand {
                id:   1,
                rows: self.group_demand,
            }],
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary => self.summary_drawn.min(area.height),
            TileContent::Group(_) => self.group_drawn.min(area.height),
            TileContent::Empty(_) => 0,
        }
    }

    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}

struct SameFrameSummaryCells {
    alone_rows: usize,
}

impl crate::TileCells<u32> for SameFrameSummaryCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        let summary_width = widths
            .iter()
            .find_map(|(content, width)| (*content == TileContent::Summary).then_some(*width))
            .unwrap_or_default();
        let summary = if summary_width < TEST_TILE_WIDTH {
            5
        } else {
            self.alone_rows
        };
        TileDemands {
            summary,
            summary_width: 0,
            groups: (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary if area.width < TEST_TILE_WIDTH => 5,
            TileContent::Summary => u16::try_from(self.alone_rows)
                .unwrap_or(u16::MAX)
                .min(area.height),
            TileContent::Group(_) | TileContent::Empty(_) => area.height,
        }
    }

    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}

struct WidthDependentSummaryCells;

impl crate::TileCells<u32> for WidthDependentSummaryCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        let summary_width = widths
            .iter()
            .find_map(|(content, width)| (*content == TileContent::Summary).then_some(*width))
            .unwrap_or_default();
        TileDemands {
            summary:       if summary_width < TEST_TILE_WIDTH {
                30
            } else {
                5
            },
            summary_width: 0,
            groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary if area.width < TEST_TILE_WIDTH => 30.min(area.height),
            TileContent::Summary => 5.min(area.height),
            TileContent::Group(_) => 1.min(area.height),
            TileContent::Empty(_) => 0,
        }
    }

    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}

type DemandMeasurementWidths = Vec<(TileContent<u32>, u16)>;

const NARROW_TREE_WIDTH: u16 = 60;

const NARROW_SUMMARY_ROWS: u16 = 28;

const WIDE_SUMMARY_ROWS: u16 = 22;

const COMPRESSED_TREE_ROWS: u16 = 4;

const FULL_TREE_ROWS: u16 = 6;

const SUMMARY_PREFERRED_WIDTH: u16 = 78;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum FramePreparation {
    #[default]
    Unprepared,
    Prepared,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum TreeHeaders {
    #[default]
    Full,
    Compressed,
}

/// A narrow tree omits its heading and requests fewer rows.
#[derive(Default)]
struct NarrowTreeDemandCells {
    prepared:           Cell<FramePreparation>,
    compressed_headers: Cell<TreeHeaders>,
    measurements:       RefCell<Vec<DemandMeasurementWidths>>,
}

impl crate::TileCells<u32> for NarrowTreeDemandCells {
    fn summary_title(&self) -> &'static str { "accounts" }

    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent<u32>, Rect)>) {
        let allocations = allocations();
        assert!(!allocations.is_empty(), "the grid prepares its allocations");
        self.compressed_headers.set(
            if allocations.iter().any(|(content, area)| {
                matches!(content, TileContent::Group(_)) && area.width < NARROW_TREE_WIDTH
            }) {
                TreeHeaders::Compressed
            } else {
                TreeHeaders::Full
            },
        );
        self.prepared.set(FramePreparation::Prepared);
    }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        assert!(
            self.prepared.get() == FramePreparation::Prepared,
            "preparation precedes every measurement"
        );
        self.measurements.borrow_mut().push(widths.to_vec());
        TileDemands {
            summary:       if widths.iter().any(|(content, width)| {
                matches!(content, TileContent::Summary) && *width < NARROW_TREE_WIDTH
            }) {
                usize::from(NARROW_SUMMARY_ROWS)
            } else {
                usize::from(WIDE_SUMMARY_ROWS)
            },
            summary_width: SUMMARY_PREFERRED_WIDTH,
            groups:        (1..=6)
                .map(|id| {
                    let width = widths
                        .iter()
                        .find_map(|(content, width)| {
                            (*content == TileContent::Group(id)).then_some(*width)
                        })
                        .unwrap_or(SUMMARY_PREFERRED_WIDTH);
                    TileDemand {
                        id,
                        rows: if width < NARROW_TREE_WIDTH
                            || self.compressed_headers.get() == TreeHeaders::Compressed
                        {
                            usize::from(COMPRESSED_TREE_ROWS)
                        } else {
                            usize::from(FULL_TREE_ROWS)
                        },
                    }
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        let rows = match content {
            TileContent::Summary if area.width < NARROW_TREE_WIDTH => NARROW_SUMMARY_ROWS,
            TileContent::Summary => WIDE_SUMMARY_ROWS,
            TileContent::Group(_)
                if area.width < NARROW_TREE_WIDTH
                    || self.compressed_headers.get() == TreeHeaders::Compressed =>
            {
                COMPRESSED_TREE_ROWS
            },
            TileContent::Group(_) => FULL_TREE_ROWS,
            TileContent::Empty(_) => 0,
        };
        rows.min(area.height)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, area: Rect, _: Color) {
        if matches!(content, TileContent::Group(_)) && area.height > 0 {
            let header = if area.width < NARROW_TREE_WIDTH
                || self.compressed_headers.get() == TreeHeaders::Compressed
            {
                "session role status"
            } else {
                "session type role status account"
            };
            buffer.set_string(area.x, area.y, header, ratatui::style::Style::default());
        }
    }
}

#[derive(Clone, Copy)]
enum SummaryLineArrangement {
    Stacked,
    Inline,
}

struct ReflowingSummaryCells {
    arrangement: Cell<SummaryLineArrangement>,
}

impl ReflowingSummaryCells {
    fn content_rows(&self, content: &TileContent<u32>, width: u16) -> usize {
        match (content, self.arrangement.get()) {
            (TileContent::Summary, SummaryLineArrangement::Stacked)
                if width < NARROW_TREE_WIDTH =>
            {
                30
            },
            (TileContent::Summary, _) => 5,
            (TileContent::Group(_), _) => 1,
            (TileContent::Empty(_), _) => 0,
        }
    }
}

impl crate::TileCells<u32> for ReflowingSummaryCells {
    fn summary_title(&self) -> &'static str { "accounts" }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        let width = widths
            .iter()
            .find_map(|(content, width)| (*content == TileContent::Summary).then_some(*width))
            .unwrap_or_default();
        TileDemands {
            summary:       self.content_rows(&TileContent::Summary, width),
            summary_width: match self.arrangement.get() {
                SummaryLineArrangement::Stacked => SUMMARY_PREFERRED_WIDTH,
                SummaryLineArrangement::Inline => SUMMARY_PREFERRED_WIDTH / 2,
            },
            groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        u16::try_from(self.content_rows(content, area.width))
            .unwrap_or(u16::MAX)
            .min(area.height)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, area: Rect, _: Color) {
        for row in 0..crate::TileCells::rows_drawn(self, content, area) {
            buffer.set_string(
                area.x,
                area.y + row,
                "row",
                ratatui::style::Style::default(),
            );
        }
    }
}

fn draw_view_frame(
    grid: &mut TileGrid<u32>,
    cells: &impl crate::TileCells<u32>,
    area: Rect,
    growth: TileGrowth,
) {
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    crate::draw_tile_grid(
        &mut buffer,
        grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
}

/// A scan of `groups` quiet cells whose summary asks for `width`
/// columns across.
fn wide_summary(groups: &[u32], width: u16) -> TileDemands<u32> {
    TileDemands {
        summary_width: width,
        ..quiet(groups)
    }
}

/// Rows per column for every count up to `count`, so a walk through
/// the sequence reads as one table.
fn walk(count: usize, growth: TileGrowth) -> Vec<Vec<usize>> {
    (1..=count).map(|n| columns(n, growth)).collect()
}

/// Row-use readings from a startup column whose transfers repeated.
#[derive(Default)]
struct StartupColumnRows {
    readings: usize,
}

impl ColumnRowReadings for StartupColumnRows {
    fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16 {
        self.readings += 1;
        assert!(self.readings <= 512, "startup transfers do not finish");
        let reading = match (index, rows) {
            (0, 25..=31) => Some((rows - 25, 0)),
            (0, 32) => Some((7, 0)),
            (1, 6..=8) | (2, 5..=7) | (3, 6..=7) => Some((0, 0)),
            (3, 8 | 10) => Some((1, 1)),
            (3, 9) => Some((2, 1)),
            _ => None,
        };
        let (retained, painted) = reading.expect("the startup fixture defines this allocation");
        match purpose {
            RowUsePurpose::RetainContent => retained,
            RowUsePurpose::PaintContent => painted,
        }
    }
}

/// Place one cell motion through the same shared-band path as a frame.
fn placed_motion(
    before: &Grid,
    after: &Grid,
    transition: CellTransition,
    progress: u32,
    drawn: CellAppearance<u32>,
) -> Vec<TilePlacement<u32>> {
    let bands: Vec<Rect> = (0..before.columns.len().max(after.columns.len()))
        .map(|column| column_band(before, after, column, progress))
        .collect();
    let mut pieces = Vec::new();
    moving_cell(before, after, transition, progress, drawn, &mut pieces);
    place_band_pieces(before, after, &bands, pieces, &HeightSnap::Off)
        .into_iter()
        .map(|piece| piece.placement)
        .collect()
}

/// Render the new widths, then inspect the one motion the key started.
fn assert_width_changing_key_moves_once(
    grid: &mut TileGrid<u32>,
    cells: &WidthDependentSummaryCells,
    area: Rect,
    growth: TileGrowth,
    action: TileAction,
    expected_depth: usize,
) {
    grid.apply(action, growth);
    assert!(matches!(grid.motion, GridMotion::Moving(_)));
    draw_view_frame(grid, cells, area, growth);
    assert!(grid.pending.is_empty(), "the key has no second motion");
    assert_eq!(grid.depth, expected_depth);

    let contents = [TileContent::Summary, TileContent::Group(1)];
    let mut paths = vec![Vec::new(); contents.len()];
    for raw in grid.distinct_motion_steps_for_test(area, growth) {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| crate::TileCells::rows_drawn(cells, content, area),
            |content, area| crate::TileCells::rows_kept(cells, content, area),
        );
        let drawing = grid.drawing_at(area, growth, raw);
        for (content, path) in contents.iter().zip(&mut paths) {
            let height = drawing
                .placements()
                .find(|placement| &placement.content == content)
                .map(|placement| placement.frame.clip().height)
                .expect("the unchanged cell remains visible");
            path.push(height);
        }
    }
    for (content, heights) in contents.iter().zip(paths) {
        let (&start, &end) = heights
            .first()
            .zip(heights.last())
            .expect("the key draws one motion");
        let one_way = if start <= end {
            heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(one_way, "{content:?} reverses through {heights:?}");
    }

    grid.advance();
    assert!(matches!(grid.motion, GridMotion::Settled));
    assert_eq!(grid.depth, expected_depth);
    draw_view_frame(grid, cells, area, growth);
}

/// Every placement assigned to a band shares both outside rows and
/// every divider between them with the piece beside it.
fn assert_shared_piece_rows(grid: &TileGrid<u32>, growth: TileGrowth) {
    for raw in motion_steps() {
        let drawing = grid.drawing_at(test_area(), growth, raw);
        for &band in &drawing.column_bands {
            let pieces: Vec<Rect> = drawing
                .placements()
                .map(|placement| placement.frame.clip())
                .filter(|rect| rect.x == band.x && rect.width == band.width)
                .collect();
            assert!(!pieces.is_empty(), "column {band:?} has no pieces at {raw}");
            assert_eq!(
                pieces[0].top(),
                band.top(),
                "first boundary at {raw}: {band:?}, {pieces:?}"
            );
            assert_eq!(
                pieces.last().map(|piece| piece.bottom()),
                Some(band.bottom())
            );
            for pair in pieces.windows(2) {
                assert_eq!(
                    pair[0].bottom() - 1,
                    pair[1].top(),
                    "shared boundary at {raw}: {band:?}, {pieces:?}"
                );
            }
        }
    }
}

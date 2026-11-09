use super::*;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::TILE_BORDER_ROWS;
use crate::tiles::constants::TILE_ROWS_READOUT_HEIGHT;
use crate::tiles::grid::tests::*;
use crate::tiles::grid::*;

/// A replacement cannot take the old invocation's cell or focus identity.
#[test]
fn reused_pids_keep_distinct_tiles_and_focus() {
    let first = (TEST_PID, u64::from(TEST_PID));
    let replacement = (TEST_PID, TEST_REPLACEMENT_LIFETIME);
    let mut demands = TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        vec![TileDemand {
            id:   first,
            rows: 0,
        }],
    };
    let mut grid = seeded_grid();
    grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
    grid.settle_for_test();
    grid.focus_cell(TABLE_CELL + 1);
    demands.groups.push(TileDemand {
        id:   replacement,
        rows: 0,
    });

    grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
    grid.settle_for_test();

    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(first), TileContent::Group(replacement)]
    );
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(first)));
    demands.groups.remove(0);
    grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
    grid.settle_for_test();
    assert_eq!(shown(&grid), vec![TileContent::Group(replacement)]);
    assert_eq!(grid.focus, Focus::Summary);
}

#[test]
fn a_click_anywhere_in_the_summary_alone_picks_the_summary() {
    let growth = redistribute(4);
    let mut grid = seeded_grid();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.sync(&quiet(&[7]), growth);
    grid.settle_for_test();
    grid.focus_cell(TABLE_CELL + 1);
    let held_focus = grid.focus.clone();
    grid.set_layout(Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT), growth);

    for point in [
        Position::new(0, 0),
        Position::new(TEST_TILE_WIDTH - 2, TEST_HEIGHT - 1),
    ] {
        assert_eq!(grid.cell_at(point), Some(TABLE_CELL));
        grid.focus_cell(TABLE_CELL);
        assert_eq!(grid.focus, held_focus);
    }
}

#[test]
fn focus_returns_to_its_cell_with_the_cells() {
    let growth = redistribute(4);
    let mut grid = seeded_grid();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.sync(&quiet(&[7]), growth);
    grid.settle_for_test();
    grid.focus_cell(TABLE_CELL + 1);
    let held_focus = grid.focus.clone();
    grid.set_layout(Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT), growth);

    assert!(!grid.cycle_focus(CycleDirection::Next));
    grid.focus_step(Direction::Up, growth);
    assert_eq!(grid.focus, held_focus);

    grid.set_layout(Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT), growth);
    let focused = grid
        .drawing_at(
            Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT),
            growth,
            PROGRESS_SCALE,
        )
        .pieces
        .into_iter()
        .find(|piece| piece.placement.frame.is_focused())
        .map(|piece| piece.placement.content);
    assert_eq!(focused, Some(TileContent::Group(7)));
}

/// Where the room does not go round, the cell being watched is the
/// one that gets it and the rest divide what is left.
#[test]
fn the_focused_cell_wins_the_room_that_does_not_go_round() {
    let share = TEST_HEIGHT / 4;
    let asked = [0, share * 2, share * 2, 0];
    let level = column_rows(&asked, ColumnFocus::Outside);
    let watched = column_rows(&asked, ColumnFocus::Row(1));
    assert!(
        level[1].abs_diff(level[2]) <= 1,
        "unwatched, the two short cells divide it between them: {level:?}"
    );
    assert!(
        watched[1] > watched[2],
        "watched, the ring is served first: {watched:?}"
    );
    assert_eq!(
        watched.iter().sum::<u16>(),
        TEST_HEIGHT,
        "and the column is still filled exactly"
    );
}

/// Focus settles nothing while there is enough room to go round:
/// every short cell is filled whether it is watched or not.
#[test]
fn focus_changes_nothing_while_there_is_room_to_go_round() {
    let share = TEST_HEIGHT / 4;
    for asked in [vec![demanded_rows(0); 4], vec![0, share + 1, share + 1, 0]] {
        let demands = row_demands(&asked);
        assert_eq!(
            shares(
                &demands,
                TEST_HEIGHT,
                ColumnHead::Cell,
                ColumnFocus::Row(1),
                MIN_TILE_HEIGHT
            ),
            shares(
                &demands,
                TEST_HEIGHT,
                ColumnHead::Cell,
                ColumnFocus::Outside,
                MIN_TILE_HEIGHT
            ),
            "nothing is competing, so the ring changes nothing: {asked:?}"
        );
    }
}

/// Focus is a claim on the room going spare rather than a right to
/// it: a column where every cell has run out hands it nothing.
#[test]
fn focus_takes_nothing_from_a_column_that_has_no_room_to_spare() {
    let asked = vec![u16::MAX; 3];
    let demands = row_demands(&asked);
    assert_eq!(
        shares(
            &demands,
            TEST_HEIGHT,
            ColumnHead::Cell,
            ColumnFocus::Row(1),
            MIN_TILE_HEIGHT
        ),
        shares(
            &demands,
            TEST_HEIGHT,
            ColumnHead::Cell,
            ColumnFocus::Outside,
            MIN_TILE_HEIGHT
        ),
        "every cell is short, so the ring changes nothing"
    );
}

#[test]
fn seven_cell_plus_and_minus_start_every_piece_one_way() {
    let area = Rect::new(0, 0, 200, 50);
    let growth = redistribute(12);
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(6);
    grid.set_layout(area, growth);
    grid.sync(&tall_demands(&[1, 2, 3, 4, 5, 6]), growth);
    grid.settle_for_test();

    assert_key_motion_starts_one_way(&mut grid, area, growth, TileAction::Add);
    grid.settle_for_test();
    assert_key_motion_starts_one_way(&mut grid, area, growth, TileAction::Remove);
}

#[test]
fn unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion() {
    let fewer = [19, 7, 8, 8, 8, 7];
    let more = [15, 9, 10, 6, 6, 6, 6];
    let groups = (1..=5).map(Slot::Group).collect::<Vec<_>>();
    let mut added = groups.clone();
    added.push(Slot::Empty(0));
    for (before, after, before_slots, after_slots) in [
        (&fewer[..], &more[..], groups.clone(), added.clone()),
        (&more[..], &fewer[..], added, groups),
    ] {
        let (mut grid, area, growth) =
            exact_slot_height_motion(before, after, before_slots, after_slots);
        let endpoints = before
            .iter()
            .copied()
            .zip(after.iter().copied())
            .collect::<Vec<_>>();
        let painted_rows = endpoint_painted_rows(&endpoints, area.height);
        let mut prior = before.to_vec();
        let mut heights = Vec::with_capacity(before.len().max(after.len()));
        let opening = grid.drawing_at_with_height_snap(area, growth, 0, &HeightSnap::Off);
        assert_eq!(opening.column_bands, [area]);
        let steps = single_column_height_steps(&opening);
        let mut raw = 0;
        loop {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, probe_area| endpoint_rows_drawn(content, probe_area, &painted_rows),
                |_, area| area.height,
            );
            if raw == PROGRESS_SCALE {
                assert_eq!(grid.motion_heights.piece_heights, [] as [u16; 0]);
            } else {
                assert_eq!(grid.motion_heights.progress, raw);
                assert_ne!(grid.motion_heights.piece_heights, [] as [u16; 0]);
            }
            let destination = grid.resolved_grid(area, &grid.drawn_held(), growth);
            assert_eq!(destination.columns, [area]);
            assert_eq!(
                destination
                    .resolved
                    .panes
                    .iter()
                    .map(|pane| pane.area.height)
                    .collect::<Vec<_>>(),
                after
            );
            heights.clear();
            heights.extend(
                grid.drawing_at(area, growth, raw)
                    .placements()
                    .map(|placement| placement.frame.clip().height),
            );
            for (index, (&height, &(start, end))) in heights.iter().zip(&endpoints).enumerate() {
                assert!(
                    (start.min(end)..=start.max(end)).contains(&height),
                    "piece {index} leaves {start}..={end} at {raw}: {heights:?}"
                );
                let moved_toward_end = if start <= end {
                    height >= prior[index]
                } else {
                    height <= prior[index]
                };
                assert!(
                    moved_toward_end,
                    "piece {index} moves away from {end} at {raw}: {prior:?} to {heights:?}"
                );
            }
            if raw == PROGRESS_SCALE {
                break;
            }
            // Stable probes repeat until the next integer transfer;
            // changing heights still advance on consecutive frames.
            raw = if heights == prior {
                steps
                    .iter()
                    .copied()
                    .find(|&step| step > raw)
                    .unwrap_or(PROGRESS_SCALE)
            } else {
                raw.saturating_add(1)
            };
            std::mem::swap(&mut prior, &mut heights);
        }
    }
}

/// The ring waits for a cell that closed to let the order through.
/// Dropping it to the summary would make the developer find the
/// command again over a change they never asked for.
#[test]
fn focus_waits_for_a_cell_that_closed_to_let_the_order_through() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();
    grid.focus_step(Direction::Down, redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

    grid.sync(&quiet(&[8, 7]), redistribute(4));
    assert_eq!(
        grid.focus,
        Focus::Cell(Slot::Group(7)),
        "the ring stays with the command while its cell is off the grid"
    );
    assert_eq!(
        grid.focused_cell(),
        FocusLocation::Departing,
        "and no cell is drawn holding it meanwhile"
    );

    grid.settle_for_test();
    assert_eq!(
        grid.focused_cell(),
        FocusLocation::Cell(TABLE_CELL + 2),
        "it comes back with the cell, one place further down"
    );
}

#[test]
fn the_summary_holds_focus_to_begin_with() {
    assert_eq!(TileGrid::<u32>::new().focus, Focus::Summary);
}

#[test]
fn focus_walks_the_grid_and_stops_at_its_edges() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(3));
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(
        grid.focused_cell(),
        FocusLocation::Cell(3),
        "the last cell is the floor"
    );
    grid.focus_step(Direction::Up, redistribute(4));
    grid.focus_step(Direction::Up, redistribute(4));
    assert_eq!(grid.focus, Focus::Summary, "and the summary is the ceiling");
}

/// Tab reads the grid as the list it is numbered as, and wraps
/// rather than stopping where the arrows stop.
#[test]
fn tab_walks_every_cell_and_comes_back_round() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    assert!(grid.cycle_focus(CycleDirection::Next));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
    assert!(grid.cycle_focus(CycleDirection::Next));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(3));
    assert!(grid.cycle_focus(CycleDirection::Next));
    assert_eq!(
        grid.focus,
        Focus::Summary,
        "the last cell wraps to the summary"
    );
    assert!(grid.cycle_focus(CycleDirection::Prev));
    assert_eq!(
        grid.focused_cell(),
        FocusLocation::Cell(3),
        "and back the other way"
    );
}

/// Focus is held by identity, so the cell it is on keeps it while
/// the grid closes up around it.
#[test]
fn focus_follows_a_cell_through_a_closing() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();
    grid.focus_step(Direction::Down, redistribute(4));
    grid.focus_step(Direction::Down, redistribute(4));
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));

    grid.sync(&quiet(&[7, 9]), redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));
    assert_eq!(
        grid.focused_cell(),
        FocusLocation::Cell(3),
        "one place forward"
    );
}

#[test]
fn a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working() {
    let growth = redistribute(4);
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[1, 2, 3]), growth);
    grid.settle_for_test();
    grid.focus_cell(TABLE_CELL + 3);
    grid.settle_for_test();

    grid.sync(&quiet(&[1]), growth);
    assert!(matches!(grid.motion, GridMotion::Moving(_)));
    assert!(!grid.pending.is_empty());
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(3)));

    let area = Rect {
        width: grid.area.width - 1,
        ..grid.area
    };
    grid.set_layout(area, growth);
    assert_eq!(shown(&grid), vec![TileContent::Group(1)]);
    assert_eq!(
        grid.focus,
        Focus::Summary,
        "the removed group releases focus"
    );
    assert_eq!(grid.held.focused, FocusLocation::Cell(TABLE_CELL));
    assert!(grid.pending.is_empty());
    assert!(matches!(grid.motion, GridMotion::Settled));

    assert!(
        grid.cycle_focus(CycleDirection::Next),
        "Tab reaches a live cell"
    );
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
    grid.apply(TileAction::FocusUp, growth);
    assert_eq!(grid.focus, Focus::Summary, "arrows reach the summary");
    grid.apply(TileAction::FocusDown, growth);
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
}

#[test]
fn focus_falls_back_to_the_summary_when_its_command_ends() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

    grid.sync(&quiet(&[]), redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.focus, Focus::Summary);
}

/// The developer opened the cell to watch for exactly this, so the
/// ring stays on it when a command lands there.
#[test]
fn focus_stays_on_an_empty_cell_a_command_claims() {
    let mut grid = seeded_grid();
    grid.add(redistribute(4));
    grid.settle_for_test();
    grid.focus_step(Direction::Down, redistribute(4));
    assert!(matches!(grid.focus, Focus::Cell(Slot::Empty(_))));

    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
}

/// A click is the other way onto a cell, and lands on the same ring
/// the arrows walk.
#[test]
fn a_click_inside_a_cell_takes_the_focus_ring() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    let second = Grid::new(
        test_area(),
        &even(grid.count()),
        redistribute(4),
        &TileSettings::default(),
    )
    .cell(2)
    .expect("the grid has three cells");
    let inside = Position::new(second.x + second.width / 2, second.y + second.height / 2);
    let index = grid.cell_at(inside).expect("the point is inside cell two");
    grid.focus_cell(index);

    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
}

#[test]
fn a_click_outside_the_grid_finds_no_cell() {
    let grid = seeded_grid::<u32>();
    assert_eq!(grid.cell_at(Position::new(TEST_WIDTH, TEST_HEIGHT)), None);
}

/// `-` takes out the cell the ring is on when `+` opened it, rather
/// than the last one opened.
#[test]
fn minus_takes_out_the_focused_empty_cell() {
    let mut grid = seeded_grid::<u32>();
    grid.add(redistribute(4));
    grid.add(redistribute(4));
    grid.settle_for_test();
    let first = grid.slots[0].clone();
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focus, Focus::Cell(first.clone()));

    grid.remove();
    grid.settle_for_test();
    assert_eq!(grid.slots.len(), 1, "one cell went");
    assert_ne!(grid.slots[0], first, "and it was the focused one");
    assert_eq!(grid.focus, Focus::Summary, "the ring falls back");
}

/// A cell carrying a command is the display itself, so `-` leaves it
/// where it is and takes the last empty cell instead.
#[test]
fn minus_leaves_a_running_command_alone() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.add(redistribute(4));
    grid.settle_for_test();
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

    grid.remove();
    grid.settle_for_test();
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(7)],
        "the command keeps its cell and the empty one goes"
    );
}

#[test]
fn minus_with_no_empty_cell_changes_nothing() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();

    grid.remove();
    grid.settle_for_test();
    assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
}

/// The pid both invocations of the reused-pid test share.
const TEST_PID: u32 = 42;

/// The lifetime that tells the replacement apart from the original,
/// whose lifetime is its pid.
const TEST_REPLACEMENT_LIFETIME: u64 = 100_002;

/// Check the first moving frame against the settled frame before one key.
fn assert_key_motion_starts_one_way(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
    action: TileAction,
) {
    let recorded = recorded_settled_heights(grid, area, growth);
    grid.apply(action, growth);
    let raw = PROGRESS_SCALE / 20;
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |content, area| match content {
            TileContent::Group(1) | TileContent::Empty(_) => 0,
            TileContent::Summary | TileContent::Group(_) => area.height,
        },
        |content, area| match content {
            TileContent::Group(1) | TileContent::Empty(_) => 0,
            TileContent::Summary | TileContent::Group(_) => area.height,
        },
    );
    let first = grid.drawing_at(area, growth, raw);
    let destination = grid.drawing_at(area, growth, PROGRESS_SCALE);
    for (content, start) in recorded {
        let Some(first) = piece_height(&first, &content) else {
            continue;
        };
        let Some(end) = piece_height(&destination, &content) else {
            continue;
        };
        assert!(
            (start.min(end)..=start.max(end)).contains(&first),
            "{content:?} moves from {start} away from its {end} destination to {first}"
        );
    }
}

/// Painted rows at every height of the endpoint-and-midpoint fixture.
fn endpoint_painted_rows(endpoints: &[(u16, u16)], height: u16) -> Vec<Vec<u16>> {
    endpoints
        .iter()
        .map(|&(start, end)| {
            let filled = [start, u16::midpoint(start, end), end];
            (0..=height)
                .map(|height| {
                    let framed = height
                        .saturating_add(TILE_BORDER_ROWS)
                        .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                    if filled.contains(&framed) {
                        height
                    } else {
                        height.saturating_sub(1)
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
}

/// The endpoint fixture's painted rows, including its unbounded probe.
fn endpoint_rows_drawn(content: &TileContent<u32>, area: Rect, painted_rows: &[Vec<u16>]) -> u16 {
    if area.height == u16::MAX {
        return TEST_HEIGHT;
    }
    let index = match content {
        TileContent::Summary => 0,
        TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
        TileContent::Empty(_) => return 0,
    };
    painted_rows
        .get(index)
        .and_then(|rows| rows.get(usize::from(area.height)))
        .copied()
        .unwrap_or_else(|| area.height.saturating_sub(1))
}

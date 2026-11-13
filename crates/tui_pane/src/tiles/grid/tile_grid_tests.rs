#[path = "ranked_column_grid_tests.rs"]
mod ranked_columns;

use super::*;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT;
use crate::tiles::constants::SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT;
use crate::tiles::grid::tests::*;
use crate::tiles::grid::*;

/// Twelve cells stand as three columns of four either way. The
/// thirteenth opens a fourth column: add new stands it there alone,
/// and redistribute deals the thirteen out as four, three, three and
/// three. The fourteenth fills the fourth column one more under add
/// new, and makes the second column four tall under redistribute.
#[test]
fn twelve_thirteen_and_fourteen_cells_under_each_fill() {
    assert_eq!(columns(12, add_new(4)), vec![4, 4, 4]);
    assert_eq!(columns(13, add_new(4)), vec![4, 4, 4, 1]);
    assert_eq!(columns(14, add_new(4)), vec![4, 4, 4, 2]);

    assert_eq!(columns(12, redistribute(4)), vec![4, 4, 4]);
    assert_eq!(columns(13, redistribute(4)), vec![4, 3, 3, 3]);
    assert_eq!(columns(14, redistribute(4)), vec![4, 4, 3, 3]);
}

/// The grid comes back down through the arrangements it went up
/// through, because the answer is a function of the count and
/// nothing else. Taking the fourth cell away leaves a column of
/// three rather than a two-by-two with a hole in it.
#[test]
fn the_grid_comes_back_down_the_way_it_went_up() {
    for fill in FILLS {
        let growth = TileGrowth {
            initial_rows: 3,
            fill,
            widen_summary: false,
        };
        assert_eq!(columns(5, growth), vec![3, 2]);
        assert_eq!(columns(4, growth), vec![2, 2]);
        assert_eq!(columns(3, growth), vec![3]);
    }
}

#[test]
fn every_arrangement_holds_exactly_the_cells_asked_for() {
    for fill in FILLS {
        for initial_rows in 1..=6 {
            for count in 1..=60 {
                let growth = TileGrowth {
                    initial_rows,
                    fill,
                    widen_summary: false,
                };
                assert_eq!(
                    columns(count, growth).iter().sum::<usize>(),
                    count,
                    "count {count} at {initial_rows} initial rows, {fill:?}"
                );
            }
        }
    }
}

/// Between one rearrangement and the next, adding a cell under add
/// new moves nothing already on the screen: it lands at the foot of
/// the last column or opens the next one. The rearrangements are
/// the only motion, which is what makes them worth watching.
#[test]
fn nothing_already_placed_moves_until_the_grid_rearranges() {
    for initial_rows in 1..=6 {
        for count in 1..60 {
            let before = columns(count, add_new(initial_rows));
            let after = columns(count + 1, add_new(initial_rows));
            // The first column stands at the height every column but
            // the last is filled to, so a change there is the
            // rearrangement itself.
            if before.first() != after.first() {
                continue;
            }
            for index in 1..=count {
                assert_eq!(
                    position(&before, index),
                    position(&after, index),
                    "cell {index} moved going from {count} to {} at {initial_rows} initial \
                         rows",
                    count + 1
                );
            }
        }
    }
}

/// The whole point of dividing a column by demand: a cell whose
/// content does not fit its even share is drawn taller than the
/// idle cells beside it.
#[test]
fn a_cell_that_has_run_out_of_room_takes_what_an_idle_one_is_not_using() {
    let share = TEST_HEIGHT / 4;
    let rows = column_rows(
        &[demanded_rows(usize::from(share) * 2), 0, 0, 0],
        ColumnFocus::Outside,
    );
    assert!(
        rows[0] > share,
        "the cell that ran out of room is given more than its share: {rows:?}"
    );
    assert!(
        rows[1].abs_diff(rows[3]) <= 1,
        "the idle cells give up the same amount as each other, give or take the row \
             integer division leaves over: {rows:?}"
    );
}

/// Room reaches a cell from either side of it. The rule this
/// replaced could only pass space downward, so a cell at the top of
/// a column could never draw on a quiet one below it.
#[test]
fn room_reaches_a_cell_from_either_side_of_it() {
    // Four cells into forty rows, so every cell's share -- and so
    // every cell's ceiling -- is the same and the two runs are
    // comparable.
    let asked = demanded_rows(usize::from(TEST_HEIGHT));
    let third = column_rows(&[0, 0, asked, 0], ColumnFocus::Outside);
    assert!(
        third[2] > third[1] && third[2] > third[3],
        "the cell draws on the neighbours above it as well as below: {third:?}"
    );
    let first = column_rows(&[asked, 0, 0, 0], ColumnFocus::Outside);
    assert_eq!(
        third[2], first[0],
        "and takes the same room wherever in the column it sits"
    );
}

/// A cell hands over every row its contents are not using, not
/// some fraction of its share: a quiet cell beside a busy one is
/// left at what it is showing.
#[test]
fn a_cell_gives_up_every_row_its_contents_do_not_need() {
    let rows = framed_column_rows(&[u16::MAX, 0, 0, 0], ColumnFocus::Outside);
    let shared = u16::try_from(rows.len().saturating_sub(1)).unwrap_or(u16::MAX);
    for quiet in &rows[1..] {
        assert_eq!(
            *quiet, MIN_TILE_HEIGHT,
            "the quiet cells keep only what a cell can be read at: {rows:?}"
        );
    }
    assert_eq!(
        rows.iter().sum::<u16>().saturating_sub(shared),
        TEST_HEIGHT,
        "and each shared row is counted by both neighbours"
    );
}

/// No cell is pushed under [`MIN_TILE_HEIGHT`], however hard its
/// neighbours are asking.
#[test]
fn no_cell_is_pushed_below_what_a_cell_can_be_read_at() {
    let rows = framed_column_rows(&[u16::MAX, u16::MAX, 0, 0], ColumnFocus::Outside);
    assert!(
        rows.iter().all(|&cell| cell >= MIN_TILE_HEIGHT),
        "the floor holds for every cell: {rows:?}"
    );
}

#[test]
fn a_cell_made_whole_gives_no_row_away() {
    let mut settled = [6, 6, 6, 5, 6];
    let exact = [17, 11, 15, 15, 12];
    let minimum = [5, 5, 5, 5, 6];

    let mut unused = measured_column_unused;
    let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
    serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

    assert_eq!(settled[0], 7);
}

/// A cell keeps the rows it grew into once its content shrinks:
/// nothing else is asking for the room, so handing it back would be
/// a resize with nothing to show for it.
#[test]
fn a_cell_keeps_what_it_grew_to_while_nothing_encroaches() {
    let mut grid = seeded_grid();
    grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));
    grid.settle_for_test();
    let grown: Vec<u16> = grid
        .held
        .rows
        .iter()
        .map(|demand| demand.stepped_framed())
        .collect();

    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();

    assert_eq!(
        grid.held
            .rows
            .iter()
            .map(|demand| demand.stepped_framed())
            .collect::<Vec<_>>(),
        grown,
        "its rows went away and it stands where it grew to"
    );
}

/// Another cell encroaching is what hands the room back. The whole
/// grid drops to what its cells are showing and divides again.
#[test]
fn a_cell_hands_its_room_back_once_another_encroaches() {
    let mut grid = seeded_grid();
    grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));
    grid.settle_for_test();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    let grown = grid.held.clone();

    grid.sync(
        &busy(&[(7, 0), (8, usize::from(TEST_HEIGHT))]),
        redistribute(4),
    );
    grid.settle_for_test();

    assert_ne!(
        grid.held, grown,
        "the cell arriving is the encroachment the grid re-divides on"
    );
    assert_eq!(
        grid.held.rows[1].stepped_framed(),
        demanded_rows(0),
        "and the cell that had grown is back to what it is showing"
    );
}

/// A stacked cell starts on the row the one above it ends on.
#[test]
fn a_row_starts_where_the_one_above_it_ends() {
    let grid = Grid::new(
        test_area(),
        &even(2),
        redistribute(4),
        &TileSettings::default(),
    );
    let first = grid.cell(1).expect("cell is in the grid");
    let second = grid.cell(2).expect("cell is in the grid");
    assert_eq!(first.bottom() - 1, second.top());
}

#[test]
fn the_table_cell_is_never_removed() {
    let mut grid = TileGrid::<u32>::new();
    grid.remove();
    assert_eq!(grid.count(), 1);
}

#[test]
fn a_grid_with_no_room_refuses_to_grow() {
    let mut grid = TileGrid::<u32>::new();
    grid.set_layout(Rect::new(0, 0, 4, 2), redistribute(4));
    grid.add(redistribute(4));
    assert_eq!(grid.count(), 1);
}

#[test]
fn three_snapped_mates_move_one_way() {
    assert_snapped_heights_move_one_way(
        &[6, 6, 10],
        &[7, 8, 7],
        &[&[6, 7], &[6, 7, 8], &[7, 9, 10]],
    );
}

#[test]
fn a_settled_frame_measures_each_cell_size_once_and_requests_no_frame() {
    let area = Rect::new(0, 0, 200, 50);
    let growth = redistribute(12);
    let demands = TileDemands {
        summary:       100,
        summary_width: 0,
        groups:        (1..=6).map(|id| TileDemand { id, rows: 100 }).collect(),
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(6);
    grid.set_layout(area, growth);
    grid.sync(&demands, growth);
    grid.settle_for_test();

    let mut drawn = Vec::new();
    let mut kept = Vec::new();
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |content, area| {
            drawn.push((content.clone(), area));
            area.height
        },
        |content, area| {
            kept.push((content.clone(), area));
            area.height
        },
    );
    assert!(kept.iter().all(|probe| drawn.contains(probe)));
    assert!(
        drawn
            .iter()
            .enumerate()
            .all(|(index, probe)| !drawn[index + 1..].contains(probe))
    );
    assert!(
        kept.iter()
            .enumerate()
            .all(|(index, probe)| !kept[index + 1..].contains(probe))
    );
    assert_eq!(drawn.len(), SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT);
    assert_eq!(kept.len(), SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT);

    grid.sync(&demands, growth);
    drawn.clear();
    kept.clear();
    grid.measure_row_steps(
        area,
        growth,
        PROGRESS_SCALE,
        |content, area| {
            drawn.push((content.clone(), area));
            area.height
        },
        |content, area| {
            kept.push((content.clone(), area));
            area.height
        },
    );

    assert!(kept.iter().all(|probe| drawn.contains(probe)));
    assert!(
        drawn
            .iter()
            .enumerate()
            .all(|(index, probe)| !drawn[index + 1..].contains(probe))
    );
    assert!(
        kept.iter()
            .enumerate()
            .all(|(index, probe)| !kept[index + 1..].contains(probe))
    );
    assert_eq!(drawn.len(), SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT);
    assert_eq!(kept.len(), SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT);
    assert!(matches!(grid.motion, GridMotion::Settled));
    assert!(!grid.tick());
}

#[test]
fn a_command_arriving_opens_its_own_cell() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
}

/// A developer who pressed `+` made room deliberately, so the next
/// build lands in it rather than opening another cell beside it.
#[test]
fn a_command_arriving_takes_the_first_empty_cell() {
    let mut grid = seeded_grid();
    grid.add(redistribute(4));
    grid.add(redistribute(4));
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();

    assert_eq!(shown(&grid).len(), 2, "no third cell opened");
    assert_eq!(shown(&grid)[0], TileContent::Group(7));
    assert_eq!(shown(&grid)[1], TileContent::Empty(3));
}

/// Only the cells carrying commands are re-ordered. An empty cell
/// is a place the reader made with `+`, and nothing shuffles
/// through it.
#[test]
fn re_ordering_leaves_an_empty_cell_where_it_was_made() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[8]), redistribute(4));
    grid.settle_for_test();
    grid.add(redistribute(4));
    grid.add(redistribute(4));
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    let shown = shown(&grid);
    assert_eq!(shown[0], TileContent::Group(7));
    assert_eq!(shown[1], TileContent::Group(8));
    assert!(
        matches!(shown[2], TileContent::Empty(_)),
        "the empty cell stayed last: {shown:?}"
    );
}

/// The cells after a departed one each move one place forward,
/// which is the travel the closing animates.
#[test]
fn a_command_leaving_the_middle_moves_the_rest_forward() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();
    grid.sync(&quiet(&[7, 9]), redistribute(4));
    grid.settle_for_test();

    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(7), TileContent::Group(9)]
    );
}

/// A command leaving the middle takes its cell with it in one
/// step: nothing is left behind to walk to the end of the grid.
#[test]
fn a_command_leaving_the_middle_closes_the_grid_in_one_step() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[7, 9]), redistribute(4));
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(7), TileContent::Group(9)],
        "the cell above it and the cell below it come together"
    );
    grid.advance();
    assert!(
        matches!(grid.motion, GridMotion::Settled),
        "and there is no second step for a hole to travel through"
    );
}

/// The order a cell stands in is not settled once it is open. A
/// driver's cell sorts by the earliest work still under it, so the
/// last of a queue finishing carries the cell past whatever started
/// in between. Reaching the new order by moving the two cells drew
/// them travelling through each other; the overtaken one closes
/// instead and comes back in behind the cell that passed it.
#[test]
fn a_cell_the_order_overtakes_closes_and_comes_back_in_behind() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[8, 7]), redistribute(4));
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8)],
        "the overtaken cell closes on its own"
    );
    grid.advance();
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8), TileContent::Group(7)],
        "and opens again behind the one that passed it"
    );
    grid.advance();
    assert!(
        matches!(grid.motion, GridMotion::Settled),
        "a close and an open, and nothing else"
    );
}

/// A cell only ever comes back in behind the cells still standing,
/// so a change in the middle of the order carries down everything
/// after it: each cell behind the overtaken one closes too, and they
/// return in the order they now read in.
#[test]
fn the_order_changing_cascades_down_the_rest_of_the_grid() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[8, 7, 9]), redistribute(4));
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8), TileContent::Group(9)],
        "the overtaken cell goes first"
    );
    grid.advance();
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8)],
        "then the cell behind it, which has nowhere left to come back in"
    );
    grid.advance();
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8), TileContent::Group(7)],
        "the overtaken cell returns behind the one that passed it"
    );
    grid.advance();
    assert_eq!(
        shown(&grid),
        vec![
            TileContent::Group(8),
            TileContent::Group(7),
            TileContent::Group(9),
        ],
        "and the last one behind that"
    );
}

/// A reorder takes no cell off the grid: whatever closed to let the
/// order through opens its own cell again rather than claiming the
/// one a reader made with `+`.
#[test]
fn a_reorder_leaves_the_cell_count_where_it_found_it() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8]), redistribute(4));
    grid.settle_for_test();
    grid.add(redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[8, 7]), redistribute(4));
    grid.settle_for_test();
    let shown = shown(&grid);
    assert_eq!(shown.len(), 3, "the grid is the size it was: {shown:?}");
    assert_eq!(shown[0], TileContent::Group(8));
    assert_eq!(shown[1], TileContent::Group(7));
    assert!(
        matches!(shown[2], TileContent::Empty(_)),
        "and the reader's cell is still there: {shown:?}"
    );
}

/// Two commands ending in the same scan do not go together. The
/// grid closes the first cell, plays that travel out, and only then
/// closes the next -- taking them in the order they stand in, which
/// is the order they arrived in. Two cells closing over each other
/// at once is a pile of lines rather than a grid coming together.
#[test]
fn commands_ending_together_close_one_cell_at_a_time() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[9]), redistribute(4));
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(8), TileContent::Group(9)],
        "the oldest of them goes first, on its own"
    );
    assert!(
        matches!(grid.motion, GridMotion::Moving(_)),
        "and its travel is in flight"
    );

    grid.advance();
    assert_eq!(
        shown(&grid),
        vec![TileContent::Group(9)],
        "the next one follows once that travel is done"
    );
}

/// Heights the frame draws from one column's disjoint layout rows.
fn framed_column_rows(wants: &[u16], focused: ColumnFocus) -> Vec<u16> {
    let mut rows = column_rows(wants, focused);
    let shared = rows.len().saturating_sub(1);
    for row in rows.iter_mut().take(shared) {
        *row = row.saturating_add(1);
    }
    rows
}

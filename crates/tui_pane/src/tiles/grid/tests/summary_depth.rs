use super::*;

fn held_at_depth(rows: Vec<u16>, summary_depth: usize) -> HeldCellLayout {
    HeldCellLayout {
        rows,
        focused: FocusLocation::Cell(TABLE_CELL),
        summary_span: 1,
        summary_depth,
    }
}

fn deep_grid() -> TileGrid<u32> {
    let mut grid = seeded_grid();
    let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
    demands.summary = 12;
    grid.sync(&demands, redistribute(4));
    grid.settle_for_test();
    grid
}

#[test]
fn the_summary_counts_as_the_fewest_cells_that_hold_it() {
    let settings = TileSettings::default();
    for (asked, depth, rows) in [
        (9, 1, 10),
        (14, 2, 27),
        (30, 2, 30),
        (35, 3, 40),
        (45, 3, 40),
    ] {
        let wants = [asked, 10, 10, 10];
        assert_eq!(
            summary_depth(test_area(), &wants, redistribute(4), &settings),
            depth
        );
        assert_eq!(
            summary_share(&wants, TEST_HEIGHT, depth, redistribute(4), MIN_TILE_HEIGHT),
            rows
        );
    }

    let idle = [30, 3, 3, 3];
    assert_eq!(
        summary_depth(test_area(), &idle, redistribute(4), &settings),
        1,
        "ordinary rebalancing lets the idle commands lend the summary their spare rows"
    );
}

#[test]
fn the_summary_gives_a_cell_back_once_it_fits_in_fewer() {
    let mut grid = seeded_grid();
    let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
    demands.summary = 33;
    grid.sync(&demands, redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.depth, 3);
    assert_eq!(grid.held.summary_depth, 3);

    demands.summary = 6;
    grid.sync(&demands, redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.depth, 1);
    assert_eq!(grid.held.summary_depth, 1);
}

#[test]
fn a_deeper_summary_pushes_every_cell_one_place_on() {
    let settings = TileSettings::default();
    let growth = redistribute(4);
    for (depth, widths) in [(1, vec![4]), (2, vec![3, 2]), (3, vec![3, 3])] {
        let held = held_at_depth(vec![10; 4], depth);
        let grid = Grid::new(test_area(), &held, growth, &settings);
        assert_eq!(grid.widths, widths);
        assert_eq!(grid.column_of(1), Some(0));
        for cell in 2..=held.rows.len() {
            assert_eq!(
                position(&grid.widths, cell + depth - 1).map(|(column, _)| column),
                grid.column_of(cell),
                "logical cell {cell} follows its position at depth {depth}"
            );
            assert!(grid.cell(cell).is_some());
        }
    }
}

#[test]
fn a_deeper_summary_tiles_its_column() {
    let held = held_at_depth(vec![30, 10, 10, 10], 2);
    let grid = Grid::new(
        test_area(),
        &held,
        redistribute(4),
        &TileSettings::default(),
    );
    let mut covered = blank_tally();
    let mut interiors = blank_tally();
    for cell in 1..=held.rows.len() {
        let rect = grid.cell(cell).expect("logical cell is in the grid");
        paint(&mut covered, rect);
        paint(&mut interiors, rect.inner(Margin::new(1, 1)));
    }
    assert!(
        covered.iter().all(|&hits| hits >= 1),
        "the grid leaves no gap"
    );
    assert!(
        interiors.iter().all(|&hits| hits <= 1),
        "interiors never overlap"
    );
    let summary = grid.cell(1).expect("summary is in the grid");
    let below = grid.cell(2).expect("cell two is below it");
    assert_eq!(summary.bottom() - 1, below.top());
}

#[test]
fn a_capped_summary_places_every_cell_without_an_empty_column() {
    let held = held_at_depth(vec![30, 10, 10, 10], 7);
    let grid = Grid::new(
        test_area(),
        &held,
        redistribute(4),
        &TileSettings::default(),
    );
    assert_eq!(grid.depth, 3);
    assert_eq!(grid.widths, vec![3, 3]);
    assert_eq!(
        grid.widths.iter().sum::<usize>(),
        held.rows.len() + grid.depth - 1
    );
    for column in 0..grid.widths.len() {
        assert!(
            (1..=held.rows.len()).any(|cell| grid.column_of(cell) == Some(column)),
            "column {column} has a cell"
        );
    }

    let mut covered = blank_tally();
    let mut interiors = blank_tally();
    for cell in 1..=held.rows.len() {
        let rect = grid.cell(cell).expect("every logical cell is placed");
        paint(&mut covered, rect);
        paint(&mut interiors, rect.inner(Margin::new(1, 1)));
    }
    assert!(
        covered.iter().all(|&hits| hits >= 1),
        "no area is left empty"
    );
    assert!(
        interiors.iter().all(|&hits| hits <= 1),
        "interiors do not overlap"
    );
}

#[test]
fn widths_follow_capped_summary_positions_before_the_next_sync() {
    let mut grid = seeded_grid();
    let mut demands = busy(&[(7, 7)]);
    demands.summary = 200;
    grid.sync(&demands, redistribute(8));
    grid.settle_for_test();
    assert_eq!(grid.held.summary_depth, 7);

    let growth = redistribute(4);
    let layout = Grid::new(test_area(), &grid.drawn_held(), growth, &grid.settings);
    assert!(layout.depth < grid.held.summary_depth);
    let widths = grid.content_widths(test_area(), growth);
    assert_eq!(widths.len(), grid.count());
    for ((_, width), cell) in widths.into_iter().zip(1..=grid.count()) {
        let rect = layout
            .cell(cell)
            .expect("cell is placed under the new growth");
        assert_eq!(
            width,
            frame_inner(rect).width,
            "cell {cell} is measured where it draws"
        );
    }
}

#[test]
fn the_summary_never_takes_the_grid_past_fits() {
    let area = Rect::new(0, 0, TEST_WIDTH, 5);
    let growth = redistribute(4);
    let settings = TileSettings::default();
    let wants = [u16::MAX, MIN_TILE_HEIGHT];
    assert!(fits(area, 2, growth, &settings));
    assert!(!fits(area, 3, growth, &settings));
    assert_eq!(summary_depth(area, &wants, growth, &settings), 1);
    assert!(summary_share(&wants, area.height, 1, growth, MIN_TILE_HEIGHT) < wants[0]);
}

#[test]
fn the_summary_is_served_before_the_focus_ring() {
    let wants = [26, 35, 0];
    let outside = shares(
        &wants,
        TEST_HEIGHT,
        ColumnHead::Summary(2),
        ColumnFocus::Outside,
        MIN_TILE_HEIGHT,
    );
    let focused = shares(
        &wants,
        TEST_HEIGHT,
        ColumnHead::Summary(2),
        ColumnFocus::Row(1),
        MIN_TILE_HEIGHT,
    );
    assert_eq!(outside[0], PaneAxisSize::Fixed(26));
    assert_eq!(
        focused[0], outside[0],
        "focus does not take the summary's spare rows"
    );
}

#[test]
fn moving_focus_never_changes_the_summary_depth() {
    let mut grid = deep_grid();
    assert_eq!(grid.depth, 2);
    for cell in [2, 3, 4, 1] {
        grid.focus_cell(cell);
        grid.settle_for_test();
        assert_eq!(grid.depth, 2, "focus on cell {cell} keeps the depth");
        assert_eq!(grid.held.summary_depth, 2);
    }
}

#[test]
fn the_arrows_move_by_position_beside_a_deep_summary() {
    let mut grid = deep_grid();
    grid.focus_step(Direction::Down, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
    grid.focus_step(Direction::Up, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
    grid.focus_cell(4);
    grid.focus_step(Direction::Left, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
    grid.focus_cell(3);
    grid.focus_step(Direction::Left, redistribute(4));
    assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
}

#[test]
fn a_click_on_any_part_of_a_deep_summary_focuses_it() {
    let mut grid = deep_grid();
    let rect = Grid::new(test_area(), &grid.held, redistribute(4), &grid.settings)
        .cell(1)
        .expect("summary is in the grid");
    let mut shallow = grid.held.clone();
    shallow.summary_depth = 1;
    let second = Grid::new(test_area(), &shallow, redistribute(4), &grid.settings)
        .cell(2)
        .expect("cell two follows a one-position summary");
    let lower = second.top() + 1;
    assert!(lower < second.bottom());
    for y in [rect.top() + 1, lower] {
        grid.focus_cell(2);
        let point = Position::new(rect.left() + 1, y);
        let clicked = grid
            .cell_at(point)
            .expect("both parts of the summary are clickable");
        assert_eq!(clicked, 1);
        grid.focus_cell(clicked);
        assert_eq!(grid.focus, Focus::Summary);
    }
}

#[test]
fn adding_under_new_growth_queues_its_summary_depth() {
    let mut grid = seeded_grid();
    let mut demands = busy(&[(7, 7)]);
    demands.summary = 200;
    grid.sync(&demands, redistribute(8));
    grid.settle_for_test();
    assert_eq!(grid.depth, 7);

    let growth = redistribute(4);
    grid.apply(TileAction::Add, growth);
    let expected = summary_depth(
        test_area(),
        &cell_wants(&grid.demands, &grid.target(), &grid.settings),
        growth,
        &grid.settings,
    );
    assert!(expected < grid.depth);
    assert_eq!(grid.target_depth(), expected);
}

#[test]
fn cell_numbers_hold_while_the_summary_deepens() {
    let mut grid = seeded_grid();
    let mut demands = busy(&[(7, 7), (8, 7)]);
    grid.sync(&demands, redistribute(4));
    grid.settle_for_test();
    grid.add(redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.depth, 1);
    let before = shown(&grid);
    assert_eq!(before[2], TileContent::Empty(4));
    demands.summary = 33;
    grid.sync(&demands, redistribute(4));
    grid.settle_for_test();
    assert_eq!(grid.depth, 3);
    assert_eq!(shown(&grid), before);
    assert_eq!(
        cells(&grid.slots)
            .into_iter()
            .map(|(_, cell)| cell)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    grid.focus_cell(2);
    assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
}

#[test]
fn deepening_travels_the_way_a_closing_cell_does_in_reverse() {
    let area = test_area();
    let settings = TileSettings::default();
    let before = Grid::new(
        area,
        &held_at_depth(vec![10; 4], 1),
        redistribute(4),
        &settings,
    );
    let after = Grid::new(
        area,
        &held_at_depth(vec![30, 10, 10, 10], 2),
        redistribute(4),
        &settings,
    );
    for progress in [
        0,
        PROGRESS_SCALE / 4,
        PROGRESS_SCALE / 2,
        PROGRESS_SCALE * 3 / 4,
        PROGRESS_SCALE,
    ] {
        let mut summary: Vec<TilePlacement<u32>> = Vec::new();
        moving_cell(
            &before,
            &after,
            (Some(1), Some(1)),
            progress,
            &drawn(),
            &mut summary,
        );
        let mut second: Vec<TilePlacement<u32>> = Vec::new();
        moving_cell(
            &before,
            &after,
            (Some(2), Some(2)),
            progress,
            &Drawn {
                content: TileContent::Empty(2),
                focused: false,
            },
            &mut second,
        );
        assert_eq!(summary.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(
            summary[0].frame.rect().bottom() - 1,
            second[0].frame.rect().top()
        );
    }
}

#[test]
fn a_steady_summary_queues_nothing() {
    let mut grid = deep_grid();
    assert_eq!(grid.depth, 2);
    assert!(grid.pending.is_empty());
    let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
    demands.summary = 12;
    grid.sync(&demands, redistribute(4));
    assert!(grid.pending.is_empty());
    assert!(matches!(grid.motion, GridMotion::Settled));
}

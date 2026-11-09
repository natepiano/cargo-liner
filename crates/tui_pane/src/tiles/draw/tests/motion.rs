use super::*;

#[test]
fn a_cell_lends_no_row_it_must_take_back_within_one_key() {
    let cells = SpareRowMotionCells;
    let growth = TileGrowth {
        initial_rows:  LENDING_TEST_INITIAL_ROWS,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let area = COLUMN_FOOT_TEST_AREA;
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.settle_for_test();
    fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.apply(TileAction::Add, growth);
    for raw in motion_snapshots().filter(|&raw| raw < PROGRESS_SCALE) {
        fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
    }
    let before = fixed_motion_drawing(
        &mut grid,
        &cells,
        area,
        growth,
        PROGRESS_SCALE.saturating_sub(1),
    );
    assert_eq!(before.column_bands.len(), 1);
    assert_a_whole_piece_stands_above_a_cutting_mate(&cells, &before);

    grid.apply(TileAction::Remove, growth);
    grid.finish_motion_for_test();
    let mut paths = before
        .placements()
        .filter(|placement| !matches!(placement.content, TileContent::Empty(_)))
        .map(|placement| {
            (
                placement.content.clone(),
                vec![placement.frame.clip().height],
            )
        })
        .collect::<Vec<_>>();
    fixed_motion_drawing(&mut grid, &cells, area, growth, 0);
    for raw in grid.distinct_motion_steps_for_test(area, growth) {
        let drawing = measured_motion_drawing(&mut grid, &cells, area, growth, raw);
        for (content, path) in &mut paths {
            let placement = drawing
                .placements()
                .find(|placement| &placement.content == content);
            assert!(
                placement.is_some(),
                "each unchanged cell remains in the column"
            );
            let Some(placement) = placement else {
                continue;
            };
            assert!(
                placement_fits(&cells, placement),
                "{content:?} leaves a gap at {raw}"
            );
            path.push(placement.frame.clip().height);
        }
    }
    for (content, mut path) in paths {
        path.dedup();
        let start = path[0];
        let end = path[path.len().saturating_sub(1)];
        if content == TileContent::Group(LENDING_TEST_WHOLE_GROUP) {
            assert_eq!([start, end], LENDING_TEST_ENDPOINT_HEIGHTS);
        }
        assert!(
            path.iter()
                .all(|height| (start.min(end)..=start.max(end)).contains(height)),
            "{content:?} leaves {start}..={end}: {path:?}"
        );
        assert!(
            path.windows(2).all(|pair| if start <= end {
                pair[0] <= pair[1]
            } else {
                pair[0] >= pair[1]
            }),
            "{content:?} reverses within one key: {path:?}"
        );
    }
}

#[test]
fn a_turn_that_grows_content_and_height_draws_no_blank_row() {
    let area = Rect::new(0, 0, 64, 29);
    let growth = TileGrowth {
        initial_rows:  12,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let before = GrowingSummaryCells { summary_rows: 6 };
    let after = GrowingSummaryCells { summary_rows: 12 };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &before,
        ColorMode::Colored,
    );
    grid.settle_for_test();

    fixed_motion_drawing(&mut grid, &after, area, growth, 0);
    for raw in grid
        .distinct_motion_steps_for_test(area, growth)
        .into_iter()
        .filter(|&raw| raw > 0 && raw < PROGRESS_SCALE)
    {
        let drawing = measured_motion_drawing(&mut grid, &after, area, growth, raw);
        let summary = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Summary);
        assert!(summary.is_some(), "the motion keeps the summary");
        let Some(summary) = summary else {
            continue;
        };
        assert!(
            placement_fits(&after, summary),
            "summary leaves a row at {raw}"
        );
    }
}

#[test]
fn the_view_holds_through_a_motion() {
    let area = Rect::new(0, 0, 77, 40);
    let min_tile_height = 14;
    let growth = TileGrowth::default();
    let quiet =
        StubCells::demanding(1, (1..=5).map(|id| (id, 1))).with_rows_drawn(TileContent::Summary, 1);
    let busy = StubCells::demanding(50, (1..=5).map(|id| (id, 1)))
        .with_rows_drawn(TileContent::Summary, 50);
    let (mut grid, _) = settled_grid_in_view(area, &quiet, TileView::Cells, min_tile_height);
    assert_eq!(grid.display(), GridDisplay::Cells);

    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &busy,
        ColorMode::Colored,
    );
    grid.set_view(TileView::Summary);
    assert_eq!(grid.display(), GridDisplay::Cells);

    grid.settle_for_test();
    buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &busy,
        ColorMode::Colored,
    );
    assert_eq!(grid.display(), GridDisplay::SummaryAlone);
}

#[test]
fn the_summary_shows_no_blank_row_while_a_cell_in_its_column_is_short() {
    let area = Rect::new(0, 0, 200, 50);
    let asked = 12;
    let cells = StubCells::demanding(asked, (1..=10).map(|id| (id, asked)));
    let (grid, _buffer) = settled_grid(area, &cells);
    let placements = grid.placements(area, TileGrowth::default());
    let summary = &placements[0];
    let summary_frame = summary.frame.rect();

    assert_eq!(summary.content, TileContent::Summary);
    assert!(placements.iter().skip(1).any(|placement| {
        let frame = placement.frame.rect();
        frame.x == summary_frame.x
            && frame.width == summary_frame.width
            && usize::from(content_area(placement.frame.inner()).height) < asked
    }));
    assert_eq!(
        usize::from(content_area(summary.frame.inner()).height),
        asked
    );
}

#[test]
fn a_cell_draws_no_spare_row_while_a_column_mate_hides_rows() {
    let area = Rect::new(0, 0, 200, 50);
    let short_ask = 1;
    let long_ask = 50;
    let cells = StubCells::demanding(
        1,
        [
            (1, 12),
            (2, 12),
            (3, short_ask),
            (4, long_ask),
            (5, 12),
            (6, 12),
            (7, 12),
            (8, 12),
        ],
    );
    let (grid, _buffer) = settled_grid(area, &cells);
    let placements = grid.placements(area, TileGrowth::default());
    let short = &placements[3];
    let long = &placements[4];

    assert_eq!(short.content, TileContent::Group(3));
    assert_eq!(long.content, TileContent::Group(4));
    assert_eq!(short.frame.rect().x, long.frame.rect().x);
    assert!(usize::from(content_area(long.frame.inner()).height) < long_ask);
    assert_eq!(
        usize::from(content_area(short.frame.inner()).height),
        short_ask
    );
}

#[test]
fn reported_unused_rows_go_to_the_most_short_column_mate() {
    let area = Rect::new(0, 0, 80, 40);
    let cells =
        StubCells::demanding(1, [(1, 100), (2, 12)]).with_rows_drawn(TileContent::Group(2), 10);
    let (grid, _buffer) = settled_grid(area, &cells);
    let placements = grid.placements(area, TileGrowth::default());
    let first = &placements[1];
    let donor = &placements[2];

    assert_eq!(first.content, TileContent::Group(1));
    assert_eq!(donor.content, TileContent::Group(2));
    assert_eq!(content_area(donor.frame.inner()).height, 10);
    assert_eq!(first.frame.rect().bottom(), donor.frame.rect().top() + 1);
}

#[test]
fn a_new_column_draws_its_whole_frame_while_its_first_cell_rises() {
    let area = Rect::new(0, 0, 12, 6);
    let columns = [Rect::new(0, 0, 7, 6), Rect::new(6, 0, 6, 6)];
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Group(7),
                PaneFrame::new(columns[0]),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(8),
                PaneFrame::new(Rect::new(6, 3, 6, 3)),
                PieceName::Shown,
            ),
        ],
        column_bands: columns.to_vec(),
    };

    let buffer = draw_motion_fixture(area, &drawing);

    assert_eq!(buffer_line(&buffer, area.top()), "┌7────┬────┐");
    assert_eq!(buffer_line(&buffer, area.bottom() - 1), "└─────┴────┘");
    for y in area.top()..area.bottom() {
        assert_ne!(buffer[(6, y)].symbol(), " ", "left side at row {y}");
        assert_ne!(buffer[(11, y)].symbol(), " ", "right side at row {y}");
    }
}

#[test]
fn a_piece_entering_the_middle_column_leaves_the_bottom_frame_whole() {
    let area = Rect::new(0, 0, 13, 6);
    let columns = [
        Rect::new(0, 0, 5, 6),
        Rect::new(4, 0, 5, 6),
        Rect::new(8, 0, 5, 6),
    ];
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Group(7),
                PaneFrame::new(columns[0]),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(8),
                PaneFrame::shifted(Rect::new(4, 3, 5, 3), 2, columns[1]),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(9),
                PaneFrame::new(columns[2]),
                PieceName::Shown,
            ),
        ],
        column_bands: columns.to_vec(),
    };

    let buffer = draw_motion_fixture(area, &drawing);

    assert_eq!(buffer_line(&buffer, area.bottom() - 1), "└───┴───┴───┘");
}

#[test]
fn an_edge_only_piece_adds_no_line_above_the_column_frame() {
    let area = Rect::new(0, 0, 6, 5);
    let drawing = TileDrawing {
        pieces:       vec![tile_piece(
            TileContent::Group(7),
            PaneFrame::shifted(Rect::new(0, 0, 6, 3), 3, area),
            PieceName::OnTheOtherPiece,
        )],
        column_bands: vec![area],
    };

    let buffer = draw_motion_fixture(area, &drawing);

    assert_eq!(
        area_lines(&buffer, area),
        ["┌────┐", "│    │", "│    │", "│    │", "└────┘"]
    );
}

#[test]
fn a_cell_arriving_at_a_column_foot_leaves_no_line_before_its_first_row() {
    let growth = TileGrowth::default();
    let cells = Toast64MotionCells {
        groups:       COLUMN_FOOT_TEST_GROUP_COUNT,
        summary_rows: COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS,
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );
    grid.settle_for_test();
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );

    for action in [
        TileAction::Add,
        TileAction::Add,
        TileAction::Remove,
        TileAction::Remove,
    ] {
        grid.apply(action, growth);
        let destination = grid.drawing_at(COLUMN_FOOT_TEST_AREA, growth, PROGRESS_SCALE);
        let foot_cell = destination.pieces.last();
        assert!(foot_cell.is_some(), "the key keeps its foot cell");
        let Some(foot_cell) = foot_cell else {
            continue;
        };
        let foot_cell = foot_cell.placement.frame;
        assert_eq!(foot_cell.clip().bottom(), COLUMN_FOOT_TEST_AREA.bottom());
        assert!(
            destination.pieces.iter().any(|piece| {
                let clip = piece.placement.frame.clip();
                clip.x == foot_cell.clip().x && clip.bottom() == foot_cell.clip().top() + 1
            }),
            "the foot cell sits under an existing cell"
        );
        assert_column_foot_has_no_doubled_line(&mut grid, &cells, growth);
        grid.finish_motion_for_test();
        fixed_motion_drawing(
            &mut grid,
            &cells,
            COLUMN_FOOT_TEST_AREA,
            growth,
            PROGRESS_SCALE,
        );
    }
}

#[test]
fn a_content_change_mid_motion_leaves_no_blank_row() {
    for changed_at in CONTENT_CHANGE_TEST_PROGRESS {
        let mut cells = ChangingSummaryCells {
            entries: CONTENT_CHANGE_TEST_ENTRIES[0],
            groups:  CONTENT_CHANGE_TEST_GROUPS,
        };
        let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
        grid.record_settled_spans_for_test(&CONTENT_CHANGE_TEST_SOURCE_SPANS);
        grid.apply(TileAction::Remove, changing_summary_growth());
        let clock = grid.motion_timing_for_test();
        let mut paths = vec![Vec::new(); usize::try_from(cells.groups).unwrap_or_default()];
        let mut last = Vec::new();
        for raw in (0..=SLIDING_TEST_INTERVALS)
            .map(|sample| sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS)
        {
            if raw >= changed_at {
                cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
            }
            let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
            assert_eq!(clock, grid.motion_timing_for_test());
            for (index, path) in paths.iter_mut().enumerate() {
                let id = u32::try_from(index.saturating_add(1)).unwrap_or_default();
                let height = drawing
                    .placements()
                    .find(|placement| placement.content == TileContent::Group(id))
                    .map(|placement| placement.frame.clip().height);
                assert!(height.is_some(), "command {id} stays visible at {raw}");
                path.extend(height);
            }
            last = column_geometry(&drawing);
        }
        for mut path in paths {
            path.dedup();
            assert!(
                path.windows(3)
                    .all(|heights| { (heights[0] < heights[1]) == (heights[1] < heights[2]) }),
                "an unchanged command reverses: {path:?}"
            );
        }
        grid.finish_motion_for_test();
        let settled = assert_motion_content_is_filled(&mut grid, cells, PROGRESS_SCALE);
        assert_eq!(
            last,
            column_geometry(&settled),
            "the key ends where it rests"
        );
        assert!(!grid.tick(), "the content change adds no later motion");
    }
}

#[test]
fn a_departed_summary_entry_gives_its_spare_rows_to_cutting_mates_at_once() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[1].saturating_add(1),
        groups:  CONTENT_CHANGE_TEST_GROUPS.saturating_add(1),
    };
    let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
    grid.record_settled_spans_for_test(&DEPARTED_ENTRY_TEST_SPANS);
    cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
    grid.apply(TileAction::Add, changing_summary_growth());
    assert_key_gives_spare_rows(&mut grid, cells);
}

#[test]
fn an_empty_departing_cell_gives_rows_along_its_slide() {
    let cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[1],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, 1);
    grid.record_settled_spans_for_test(&EMPTY_DONOR_TEST_SPANS);
    grid.apply(TileAction::Remove, changing_summary_growth());
    let first = assert_motion_content_is_filled(&mut grid, cells, 0);
    assert_eq!(
        first
            .placements()
            .find(|piece| matches!(piece.content, TileContent::Empty(_)))
            .map(|piece| piece.frame.clip().height),
        EMPTY_DONOR_TEST_SPANS.last().map(|span| span + 1)
    );
    assert_key_gives_spare_rows(&mut grid, cells);
}

#[test]
fn a_command_growing_before_a_key_keeps_its_drawn_rows() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  1,
    };
    let growth = changing_summary_growth();
    let area = COLUMN_FOOT_TEST_AREA;
    let raw = CONTENT_CHANGE_TEST_PROGRESS[0];
    let mut grid = changing_summary_grid(cells, 0);
    cells.groups = cells.groups.saturating_add(1);
    let before = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
    let clock = grid.motion_timing_for_test();
    let command = TileContent::Group(cells.groups);
    let start = before
        .placements()
        .find(|piece| piece.content == command)
        .map_or(0, |piece| piece.frame.clip().height);
    cells.groups = cells.groups.saturating_add(1);
    let widths = measurement_widths(&grid, area, growth, GridDisplay::Cells);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, growth);
    grid.apply(TileAction::Add, growth);
    let mut heights = vec![start];
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let progress = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if progress < raw {
            continue;
        }
        let drawing = assert_motion_content_is_filled(&mut grid, cells, progress);
        assert_eq!(clock, grid.motion_timing_for_test());
        heights.push(
            drawing
                .placements()
                .find(|piece| piece.content == command)
                .map_or(0, |piece| piece.frame.clip().height),
        );
    }
    for _ in 0..CONTENT_CHANGE_TEST_EMPTY_CELLS {
        grid.finish_motion_for_test();
        if !grid.tick() {
            break;
        }
        for sample in 0..=SLIDING_TEST_INTERVALS {
            let progress = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
            let drawing = assert_motion_content_is_filled(&mut grid, cells, progress);
            heights.push(
                drawing
                    .placements()
                    .find(|piece| piece.content == command)
                    .map_or(0, |piece| piece.frame.clip().height),
            );
        }
    }
    assert!(
        heights.windows(2).all(|h| h[0] <= h[1]),
        "the arriving command gives rows back: {heights:?}"
    );
}

#[test]
fn a_content_change_keeps_the_motion_clock_with_a_key_waiting() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
    grid.record_settled_spans_for_test(&CONTENT_CHANGE_TEST_SOURCE_SPANS);
    grid.apply(TileAction::Remove, changing_summary_growth());
    grid.apply(TileAction::Add, changing_summary_growth());
    let clock = grid.motion_timing_for_test();
    let demand = grid.destination_summary_demand_for_test();
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[1] {
            cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
        }
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[1] {
            assert_ne!(
                demand,
                grid.destination_summary_demand_for_test(),
                "the active key receives the new demand"
            );
        }
        assert_eq!(clock, grid.motion_timing_for_test());
        last = column_geometry(&drawing);
    }
    grid.finish_motion_for_test();
    let next = assert_motion_content_is_filled(&mut grid, cells, 0);
    assert_eq!(
        last,
        column_geometry(&next),
        "the queued key begins at the preceding endpoint"
    );
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        last = column_geometry(&assert_motion_content_is_filled(&mut grid, cells, raw));
    }
    grid.finish_motion_for_test();
    let settled = assert_motion_content_is_filled(&mut grid, cells, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert!(!grid.tick(), "only the two keys move");
}

#[test]
fn a_content_retarget_that_changes_columns_keeps_its_clock_and_filled_rows() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut growth = changing_summary_growth();
    growth.initial_rows = CONTENT_CHANGE_TEST_COLUMN_ROWS;
    let area = COLUMN_FOOT_TEST_AREA;
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.settle_for_test();
    for _ in 0..CONTENT_CHANGE_TEST_EMPTY_CELLS {
        fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
        grid.apply(TileAction::Add, growth);
        grid.settle_for_test();
    }
    let before = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.apply(TileAction::Remove, growth);
    let clock = grid.motion_timing_for_test();
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[0] {
            cells.entries = CONTENT_CHANGE_TEST_COLUMN_ENTRIES;
        }
        let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
        assert_no_unusable_row(&cells, &drawing, 0, raw);
        assert_eq!(clock, grid.motion_timing_for_test());
        assert!(
            drawing
                .placements()
                .all(|piece| area.intersection(piece.frame.clip()) == piece.frame.clip())
        );
        last = column_geometry(&drawing);
    }
    assert!(
        last.iter().any(|(content, rect)| before
            .placements()
            .find(|piece| &piece.content == content)
            .is_some_and(|piece| piece.frame.clip().x != rect.x)),
        "content changed the column membership"
    );
    grid.finish_motion_for_test();
    let settled = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert!(!grid.tick());
}

#[test]
fn a_widening_summary_keeps_every_piece_inside_its_moving_band() {
    assert_width_retarget_tiles_bands(false);
}

#[test]
fn a_narrowing_summary_keeps_every_piece_inside_its_moving_band() {
    assert_width_retarget_tiles_bands(true);
}

#[test]
fn a_wider_summary_discards_a_destination_from_different_columns() {
    let mut cells = WideningSummaryCells {
        content: ChangingSummaryCells {
            entries: CONTENT_CHANGE_TEST_ENTRIES[0],
            groups:  CONTENT_CHANGE_TEST_GROUPS,
        },
        width:   0,
    };
    let growth = TileGrowth {
        initial_rows: CONTENT_CHANGE_TEST_COLUMN_ROWS,
        widen_summary: true,
        ..changing_summary_growth()
    };
    let area = COLUMN_FOOT_TEST_AREA;
    let raw = CONTENT_CHANGE_TEST_PROGRESS[0];
    let mut grid = cached_widening_summary_grid(&cells, growth);
    let clock = grid.motion_timing_for_test();
    let rows = cells.demands(&[]);
    cells.width = area.width;
    let changed = cells.demands(&[]);
    assert_eq!(rows.summary, changed.summary);
    assert_eq!(rows.groups, changed.groups);
    let widths = measurement_widths(&grid, area, growth, GridDisplay::Cells);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync_rendered_demands(&demands, growth);
    let endpoint = grid.drawing_at(area, growth, PROGRESS_SCALE);
    assert_eq!(
        endpoint
            .placements()
            .find(|piece| piece.content == TileContent::Summary)
            .map(|piece| piece.frame.clip().width),
        Some(area.width)
    );
    let next = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
    assert!(!grid.has_retargeted_destination_for_test());
    for piece in next
        .placements()
        .filter(|piece| piece.content != TileContent::Summary)
    {
        let clip = piece.frame.clip();
        assert!(
            next.column_bands
                .iter()
                .any(|band| band.intersection(clip) == clip),
            "{raw}: {clip:?} leaves {:?}",
            next.column_bands
        );
    }
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let progress = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if progress < raw {
            continue;
        }
        let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, progress);
        assert!(!grid.has_retargeted_destination_for_test());
        assert_eq!(clock, grid.motion_timing_for_test());
        last = column_geometry(&drawing);
    }
    assert_eq!(
        last.iter()
            .find(|(content, _)| *content == TileContent::Summary)
            .map(|(_, rect)| rect.width),
        Some(area.width)
    );
    grid.finish_motion_for_test();
    let settled = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
}

#[test]
fn a_command_claims_its_empty_cell_without_hiding_a_departing_mate() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
    grid.apply(TileAction::Remove, changing_summary_growth());
    let raw = CONTENT_CHANGE_TEST_PROGRESS[0];
    let before = fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        changing_summary_growth(),
        raw,
    );
    cells.groups = cells.groups.saturating_add(1);
    cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
    let after = fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        changing_summary_growth(),
        raw,
    );
    assert!(
        after
            .placements()
            .any(|piece| piece.content == TileContent::Group(cells.groups)),
        "the arriving command draws in its claimed cell on this frame"
    );
    assert_eq!(
        column_geometry(&before).len(),
        column_geometry(&after).len(),
        "claiming an empty cell preserves every visible piece"
    );
    assert!(
        after
            .placements()
            .filter(|piece| has_visible_body_row(piece.frame)
                && matches!(piece.content, TileContent::Empty(_)))
            .count()
            > 0,
        "the unclaimed cell still slides out"
    );
}

#[test]
fn a_rendered_claim_joins_the_motion_after_an_independent_scan() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
    let growth = changing_summary_growth();
    grid.apply(TileAction::Remove, growth);
    let raw = CONTENT_CHANGE_TEST_PROGRESS[0];
    assert_motion_content_is_filled(&mut grid, cells, raw);
    let clock = grid.motion_timing_for_test();
    cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
    cells.groups = cells.groups.saturating_add(1);
    let widths = measurement_widths(&grid, COLUMN_FOOT_TEST_AREA, growth, GridDisplay::Cells);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, growth);
    let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
    assert_eq!(clock, grid.motion_timing_for_test());
    assert!(
        drawing
            .placements()
            .any(|piece| piece.content == TileContent::Group(cells.groups))
    );
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let progress = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if progress >= raw {
            assert_motion_content_is_filled(&mut grid, cells, progress);
        }
    }
    grid.finish_motion_for_test();
    assert!(!grid.tick(), "the measured claim leaves no queued handover");
}

#[test]
fn a_key_that_adds_or_removes_a_cell_slides_its_column() {
    let cells = SlidingColumnCells;
    let growth = TileGrowth {
        initial_rows:  SLIDING_TEST_INITIAL_ROWS,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );
    grid.settle_for_test();
    for action in [
        TileAction::Add,
        TileAction::Add,
        TileAction::Remove,
        TileAction::Remove,
    ] {
        assert_key_slides_column(&mut grid, &cells, growth, action);
    }
}

#[test]
fn the_first_cell_below_a_summary_slides_in_and_out() {
    let cells = Toast64MotionCells {
        groups:       0,
        summary_rows: COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS,
    };
    let growth = TileGrowth::default();
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );
    grid.settle_for_test();
    for action in [TileAction::Add, TileAction::Remove] {
        assert_key_slides_column(&mut grid, &cells, growth, action);
    }
}

#[test]
fn the_last_cell_leaves_in_steps_with_its_narrowing_column() {
    let cells = Toast64MotionCells {
        groups:       0,
        summary_rows: COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS,
    };
    let growth = TileGrowth {
        initial_rows: 1,
        ..TileGrowth::default()
    };
    let area = COLUMN_FOOT_TEST_AREA;
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.settle_for_test();
    for _ in 0..2 {
        grid.apply(TileAction::Add, growth);
        grid.settle_for_test();
    }
    let before = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    assert_eq!(before.column_bands.len(), 2);
    let foot = before.pieces.last();
    assert!(foot.is_some(), "the closing column holds its last cell");
    let Some(foot) = foot else {
        return;
    };
    let full = foot.placement.frame.clip();
    grid.apply(TileAction::Remove, growth);
    let mut widths = Vec::new();
    for sample in 0..SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
        if let Some(placement) = drawing
            .placements()
            .find(|placement| placement.content == foot.placement.content)
        {
            let clip = placement.frame.clip();
            assert_eq!(
                clip.height, full.height,
                "a closing column keeps its cell heights"
            );
            widths.push(clip.width);
        }
    }
    widths.dedup();
    assert!(widths.iter().any(|&width| width > 0 && width < full.width));
    assert!(widths.len().saturating_sub(1) >= SLIDING_TEST_MIN_CHANGES);
    assert!(widths.windows(2).all(|pair| pair[0] >= pair[1]));
}

#[test]
fn moving_tiles_keep_title_segments_together_and_inside_their_width() {
    for (width, title) in [(18, "worker · impl"), (14, "worker · i… ")] {
        let area = Rect::new(0, 0, width, 8);
        for y in [0_u16, 1, 2] {
            let shift = i32::from(y);
            let drawing = TileDrawing {
                pieces:       vec![tile_piece(
                    TileContent::Group(7),
                    PaneFrame::shifted(Rect::new(0, 0, width, 4), shift, Rect::new(0, y, width, 4)),
                    PieceName::Shown,
                )],
                column_bands: vec![area],
            };
            let mut buffer = Buffer::empty(area);
            draw_placements(
                &mut buffer,
                area,
                &drawing,
                &TileMeasurements {
                    demands: &title_demands(&[7]),
                    widths:  &[(TileContent::Group(7), width - 2)],
                },
                TileGridContents::Shown,
                &SegmentedTitleCells,
                ColorMode::Colored,
            );

            assert!(buffer_line(&buffer, y).contains(title));
            assert_eq!(buffer_text(&buffer).matches(title).count(), 1);
            assert_eq!(buffer[(1, y)].fg, Color::Blue);
            assert_eq!(buffer[(8, y)].fg, Color::Yellow);
            assert!(is_lattice_glyph(buffer[(width - 1, y)].symbol()));
        }
    }
}

#[test]
fn a_title_is_on_screen_once_while_its_cell_moves_right_and_columns_stay() {
    let before = (20..=35).collect::<Vec<_>>();
    let after = (20..=34).collect::<Vec<_>>();
    assert_titles_once_through(
        "right crossing while columns stay",
        &before,
        &after,
        STAYING_GROWTH,
    );
}

#[test]
fn a_title_is_on_screen_once_while_its_cell_moves_right_and_a_column_closes() {
    let before = (20..=25).collect::<Vec<_>>();
    assert_titles_once_through(
        "right crossing while a column closes",
        &before,
        &[20, 22, 21, 23, 24],
        CLOSING_GROWTH,
    );
}

#[test]
fn a_title_is_on_screen_once_while_its_cell_moves_left_and_its_column_closes() {
    let before = (20..=25).collect::<Vec<_>>();
    assert_titles_once_through(
        "left crossing while the source column closes",
        &before,
        &[20, 21, 22, 23, 25],
        CLOSING_GROWTH,
    );
}

#[test]
fn a_title_is_on_screen_once_while_its_cell_moves_left_and_columns_stay() {
    let before = (20..=34).collect::<Vec<_>>();
    let after = (20..=35).collect::<Vec<_>>();
    assert_titles_once_through(
        "left crossing while columns stay",
        &before,
        &after,
        STAYING_GROWTH,
    );
}

#[test]
fn an_empty_cell_number_is_on_screen_once_while_it_moves_right_and_columns_stay() {
    let before = (20..=35).collect::<Vec<_>>();
    let after = (20..=34).collect::<Vec<_>>();
    assert_empty_numbers_once_through(
        "right crossing while columns stay",
        &before,
        &after,
        STAYING_GROWTH,
    );
}

#[test]
fn an_empty_cell_number_is_on_screen_once_while_it_moves_right_and_a_column_closes() {
    let before = (20..=25).collect::<Vec<_>>();
    let grid = collapsed_title_motion(TITLE_AREA, &before, &[20, 22, 21, 23, 24], CLOSING_GROWTH);
    assert_empty_numbers_once(
        "right crossing while a column closes",
        &grid,
        CLOSING_GROWTH,
    );
}

#[test]
fn an_empty_cell_number_is_on_screen_once_while_it_moves_left_and_its_column_closes() {
    let before = (20..=25).collect::<Vec<_>>();
    assert_empty_numbers_once_through(
        "left crossing while the source column closes",
        &before,
        &[20, 21, 22, 23, 25],
        CLOSING_GROWTH,
    );
}

#[test]
fn an_empty_cell_number_is_on_screen_once_while_it_moves_left_and_columns_stay() {
    let before = (20..=34).collect::<Vec<_>>();
    let after = (20..=35).collect::<Vec<_>>();
    assert_empty_numbers_once_through(
        "left crossing while columns stay",
        &before,
        &after,
        STAYING_GROWTH,
    );
}

#[test]
fn a_departing_cell_keeps_its_title_while_its_border_is_visible() {
    let area = Rect::new(0, 0, 80, 24);
    let groups = |ids: &[u32]| TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
    };
    let motion = |before: &[u32], after: &[u32], growth| {
        let mut grid = TileGrid::new();
        grid.set_layout(area, growth);
        grid.sync(&groups(before), growth);
        grid.settle_for_test();
        grid.sync(&groups(after), growth);
        grid
    };
    let staying_growth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let closing_growth = TileGrowth {
        initial_rows:  3,
        fill:          TileFill::AddNew,
        widen_summary: false,
    };

    for (name, grid, growth, departing) in [
        (
            "staying column",
            motion(&[20, 21, 22], &[20, 21], staying_growth),
            staying_growth,
            22,
        ),
        (
            "closing column",
            motion(
                &[20, 21, 22, 23, 24, 25],
                &[20, 21, 22, 23, 24],
                closing_growth,
            ),
            closing_growth,
            25,
        ),
    ] {
        let title = group_title(departing);
        for raw in motion_snapshots() {
            let drawing = grid.drawing_at(area, growth, raw);
            assert_eq!(
                drawing
                    .pieces
                    .iter()
                    .filter(|piece| { piece.placement.content == TileContent::Group(departing) })
                    .count(),
                1,
                "the departing piece remains in the drawing"
            );
            for piece in drawing
                .pieces
                .iter()
                .filter(|piece| piece.placement.content == TileContent::Group(departing))
            {
                let clip = piece.placement.frame.clip().intersection(area);
                if !clip.is_empty() {
                    assert_eq!(piece.name, PieceName::Shown, "{name} at raw {raw}");
                }
                let title_width = u16::try_from(title.len()).unwrap_or(u16::MAX);
                if clip.height < 2 || clip.width < title_width.saturating_add(2) {
                    continue;
                }
                let buffer = draw_motion_fixture_with_width(area, &drawing, 64);
                let screen = area_lines(&buffer, area).concat();
                assert_eq!(
                    screen.matches(&title).count(),
                    1,
                    "{name} at raw {raw}; frame={:?}",
                    piece.placement.frame
                );
            }
        }
    }
}

#[test]
fn no_line_is_doubled_through_a_transition() {
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 60, 18);
    let mut failures = Vec::new();
    for (motion, opening) in [("opening", true), ("closing", false)] {
        let (grid, growth) = column_transition(area, opening);
        for raw in motion_snapshots() {
            let drawing = grid.drawing_at(area, growth, raw);
            let buffer = draw_motion_fixture(area, &drawing);
            if let Some(failure) = doubled_line_failure(&buffer, area, motion, raw) {
                let frames: Vec<_> = drawing
                    .placements()
                    .map(|placement| (&placement.content, placement.frame))
                    .collect();
                failures.push(format!(
                    "{failure}; bands={:?}; frames={frames:?}",
                    drawing.column_bands
                ));
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn two_cells_entering_a_column_never_double_a_frame_line() {
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 80, 10);
    let (grid, growth) = column_transition(area, true);
    for raw in motion_snapshots() {
        let drawing = grid.drawing_at(area, growth, raw);
        let buffer = draw_motion_fixture(area, &drawing);
        assert_eq!(
            doubled_line_failure(&buffer, area, "two head arrivals", raw),
            None,
            "bands={:?}; frames={:?}",
            drawing.column_bands,
            drawing
                .placements()
                .map(|placement| (&placement.content, placement.frame))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn sliding_pieces_leave_no_band_row_on_the_screen_ground() {
    crate::set_transparent_background(false);
    crate::set_focused_pane_tint(true);
    let area = Rect::new(0, 0, 200, 50);
    let mut failures = Vec::new();
    for (motion, opening) in [
        ("right crossing and opening", true),
        ("left crossing and closing", false),
    ] {
        let (grid, growth) = column_transition(area, opening);
        for raw in motion_snapshots() {
            let drawing = grid.drawing_at(area, growth, raw);
            let buffer = draw_motion_fixture_with_width(area, &drawing, 64);
            if let Some(failure) = bare_band_row_failure(&buffer, &drawing, motion, raw) {
                failures.push(failure);
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_sliding_arrival_shows_its_nearest_content_row_with_its_first_visible_row() {
    crate::set_transparent_background(false);
    crate::set_focused_pane_tint(true);
    let area = Rect::new(0, 0, 200, 50);
    let mut saw_above = false;
    let mut saw_below = false;
    let mut failures = Vec::new();
    for (motion, opening) in [("opening column", true), ("closing old column", false)] {
        let (grid, growth) = column_transition(area, opening);
        for raw in motion_snapshots() {
            let drawing = grid.drawing_at(area, growth, raw);
            let buffer = draw_motion_fixture_with_width(area, &drawing, 64);
            for placement in drawing.placements() {
                if drawing
                    .placements()
                    .filter(|other| other.content == placement.content)
                    .count()
                    < 2
                    || visible_body_rows(placement.frame).is_none()
                {
                    continue;
                }
                saw_above |= placement.frame.shift() < 0;
                saw_below |= placement.frame.shift() > 0;
            }
            if let Some(failure) = missing_sliding_content(&buffer, &drawing, motion, raw) {
                failures.push(failure);
                break;
            }
        }
    }
    assert!(saw_above, "the fixtures include a piece sliding from above");
    assert!(saw_below, "the fixtures include a piece sliding from below");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_command_refused_for_height_opens_when_there_is_room() {
    let growth = TileGrowth::default();
    let cells = StubCells::new([1, 2]);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let floor = MIN_TILE_HEIGHT.saturating_add(MIN_TILE_HEIGHT);
    grid.set_min_tile_height(floor);
    let short = Rect::new(0, 0, 100, floor);
    let mut buffer = Buffer::empty(short);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        short,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    assert_eq!(grid.placements(short, growth).len(), 1);

    let cell_count =
        u16::try_from(cells.demands.groups.len().saturating_add(1)).unwrap_or(u16::MAX);
    let shared_borders = cell_count.saturating_sub(1);
    let tall_height = floor
        .saturating_mul(cell_count)
        .saturating_sub(shared_borders);
    let tall = Rect::new(0, 0, 100, tall_height);
    buffer = Buffer::empty(tall);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        tall,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    assert_eq!(grid.placements(tall, growth).len(), 3);
}

#[test]
fn a_closing_column_in_flight_never_draws_a_cell_under_the_floor() {
    let growth = TileGrowth {
        initial_rows:  1,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let before = StubCells::new(1..=8);
    let after = StubCells::new(1..=3);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let mut buffer = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &before,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    grid.collapse_queued_changes_for_test();
    // No core-width tile fits below the configured tile floor.
    let area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TITLE_AREA.height);
    buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &after,
        ColorMode::Colored,
    );

    for raw in motion_snapshots() {
        assert_ne!(
            grid.drawing_at(area, growth, raw).pieces,
            [] as [crate::tiles::grid::TilePiece<u32>; 0]
        );
        assert_eq!(grid.display(), GridDisplay::SummaryAlone);
    }
    assert!(!buffer_text(&buffer).contains("group body"));
}

#[test]
fn a_depth_change_in_flight_never_draws_a_cell_under_the_floor() {
    let growth = TileGrowth {
        initial_rows:  1,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_layout(TITLE_AREA, growth);
    grid.sync(
        &TileDemands {
            summary:       200,
            summary_width: 0,
            groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
        },
        growth,
    );
    grid.settle_for_test();
    grid.collapse_queued_changes_for_test();
    // No core-width tile fits below the configured tile floor.
    let area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TITLE_AREA.height);
    grid.set_layout(area, growth);
    grid.sync(
        &TileDemands {
            summary:       1,
            summary_width: 0,
            groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
        },
        growth,
    );
    let cells = StubCells::new(1..=5);
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );

    for raw in motion_snapshots() {
        assert_ne!(
            grid.drawing_at(area, growth, raw).pieces,
            [] as [crate::tiles::grid::TilePiece<u32>; 0]
        );
        assert_eq!(grid.display(), GridDisplay::SummaryAlone);
    }
    assert!(!buffer_text(&buffer).contains("group body"));
}

const CLOSING_GROWTH: TileGrowth = TileGrowth {
    initial_rows:  3,
    fill:          TileFill::AddNew,
    widen_summary: false,
};

struct GrowingSummaryCells {
    summary_rows: usize,
}

impl TileCells<u32> for GrowingSummaryCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       self.summary_rows,
            summary_width: 0,
            groups:        (1..=4).map(|id| TileDemand { id, rows: 1 }).collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary if self.summary_rows <= 6 => 6.min(area.height),
            TileContent::Summary if area.height >= 12 => 12,
            TileContent::Summary if area.height >= 7 => 6,
            TileContent::Summary | TileContent::Group(_) => area.height,
            TileContent::Empty(_) => 0,
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

/// A moving piece whose nearest content row has not moved with its slot.
fn missing_sliding_content(
    buffer: &Buffer,
    drawing: &TileDrawing<u32>,
    motion: &str,
    raw: u32,
) -> Option<String> {
    for placement in drawing.placements() {
        if placement.frame.shift() == 0
            || !matches!(placement.content, TileContent::Group(_))
            || drawing
                .placements()
                .filter(|other| other.content == placement.content)
                .count()
                < 2
        {
            continue;
        }
        let Some(rows) = visible_body_rows(placement.frame) else {
            continue;
        };
        let inner = frame_inner(placement.frame.clip());
        let text = rows
            .clone()
            .flat_map(|y| (inner.left()..inner.right()).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        let visible_width = if placement.frame.shift() < 0 {
            inner.width.saturating_sub(TILE_ROWS_RIGHT_INSET)
        } else {
            inner.width
        };
        let visible_nearest = if placement.frame.shift() < 0 {
            let cell_inner = placement.frame.inner();
            let Some(line) = rows_readout_lines(cell_inner, 0, 64)
                .into_iter()
                .find(|line| line.width() <= usize::from(visible_width))
            else {
                continue;
            };
            line_text(&line)
        } else {
            "group body"
                .chars()
                .take(usize::from(visible_width))
                .collect::<String>()
        };
        if !text.contains(&visible_nearest) {
            return Some(format!(
                "{motion} at raw {raw} omits {visible_nearest:?} from {:?}; rect={:?}, shift={}, \
                     clip={:?}, rows={rows:?}, text={text:?}",
                placement.content,
                placement.frame.rect(),
                placement.frame.shift(),
                placement.frame.clip()
            ));
        }
    }
    None
}

/// One whole command with spare rows beside a command that is still cut.
struct SpareRowMotionCells;

impl TileCells<u32> for SpareRowMotionCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(LENDING_TEST_DEMANDS[0]),
            summary_width: 0,
            groups:        LENDING_TEST_DEMANDS
                .iter()
                .enumerate()
                .skip(1)
                .map(|(index, &rows)| TileDemand {
                    id:   u32::try_from(index).unwrap_or(u32::MAX),
                    rows: usize::from(rows),
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        let index = match content {
            TileContent::Summary => 0,
            TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
            TileContent::Empty(_) => return 0,
        };
        let rows = LENDING_TEST_DRAWN[index].min(area.height);
        if *content == TileContent::Group(LENDING_TEST_WHOLE_GROUP)
            && area.height == LENDING_TEST_GAP_HEIGHT
        {
            rows.saturating_sub(1)
        } else {
            rows
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

/// Confirm that the fixture reaches the spare-row search's donor and mate case.
fn assert_a_whole_piece_stands_above_a_cutting_mate(
    cells: &SpareRowMotionCells,
    drawing: &TileDrawing<u32>,
) {
    let whole = drawing
        .placements()
        .find(|placement| placement.content == TileContent::Group(LENDING_TEST_WHOLE_GROUP));
    assert!(
        whole.is_some(),
        "the whole command is above its cutting mate"
    );
    let Some(whole) = whole else {
        return;
    };
    let cut = drawing
        .placements()
        .find(|placement| placement.content == TileContent::Group(LENDING_TEST_CUT_GROUP));
    assert!(cut.is_some(), "the cutting mate is above the foot cell");
    let Some(cut) = cut else {
        return;
    };
    let whole_area = content_area(whole.frame.inner());
    let cut_area = content_area(cut.frame.inner());
    assert!(whole_area.height > cells.rows_kept(&whole.content, whole_area));
    assert!(
        cells.rows_drawn(&cut.content, cut_area)
            < cells.rows_drawn(
                &cut.content,
                Rect {
                    height: u16::MAX,
                    ..cut_area
                },
            )
    );
    assert!(whole_area.y < cut_area.y);
}

/// Whole command cells beside commands whose content is still cut.
struct SlidingColumnCells;

impl TileCells<u32> for SlidingColumnCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(SLIDING_TEST_ROWS[0]),
            summary_width: 0,
            groups:        SLIDING_TEST_ROWS
                .iter()
                .enumerate()
                .skip(1)
                .map(|(index, &rows)| TileDemand {
                    id:   u32::try_from(index).unwrap_or(u32::MAX),
                    rows: usize::from(rows),
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        let index = match content {
            TileContent::Summary => 0,
            TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
            TileContent::Empty(_) => return 0,
        };
        SLIDING_TEST_ROWS
            .get(index)
            .copied()
            .unwrap_or_default()
            .min(area.height)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

fn assert_key_gives_spare_rows(grid: &mut TileGrid<u32>, cells: ChangingSummaryCells) {
    let clock = grid.motion_timing_for_test();
    let mut last = Vec::new();
    let mut paths = Vec::<(TileContent<u32>, Vec<u16>)>::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        let drawing = assert_motion_content_is_filled(grid, cells, raw);
        let endpoint = grid.drawing_at(
            COLUMN_FOOT_TEST_AREA,
            changing_summary_growth(),
            PROGRESS_SCALE,
        );
        assert_eq!(clock, grid.motion_timing_for_test());
        assert_drawing_tiles_bands(&drawing, raw);
        assert_no_givable_spare_row(&cells, &drawing, &endpoint, raw);
        for piece in drawing.placements() {
            if let Some((_, heights)) = paths
                .iter_mut()
                .find(|(content, _)| *content == piece.content)
            {
                heights.push(piece.frame.clip().height);
            } else {
                paths.push((piece.content.clone(), vec![piece.frame.clip().height]));
            }
        }
        last = column_geometry(&drawing);
    }
    for (content, mut heights) in paths {
        heights.dedup();
        assert!(
            heights.windows(3).all(|h| (h[0] < h[1]) == (h[1] < h[2])),
            "{content:?} reverses: {heights:?}"
        );
    }
    grid.finish_motion_for_test();
    let settled = assert_motion_content_is_filled(grid, cells, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert_no_givable_spare_row(&cells, &settled, &settled, PROGRESS_SCALE);
    assert!(!grid.tick());
}

struct WideningSummaryCells {
    content: ChangingSummaryCells,
    width:   u16,
}

impl TileCells<u32> for WideningSummaryCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary_width: self.width,
            ..self.content.demands(widths)
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        self.content.rows_drawn(content, area)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, area: Rect, color: Color) {
        self.content.draw(buffer, content, area, color);
    }
}

/// A running removal with a measured destination retained across frames.
fn cached_widening_summary_grid(cells: &WideningSummaryCells, growth: TileGrowth) -> TileGrid<u32> {
    let area = COLUMN_FOOT_TEST_AREA;
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(&mut grid, cells, area, growth, PROGRESS_SCALE);
    grid.settle_for_test();
    for _ in 0..CONTENT_CHANGE_TEST_EMPTY_CELLS {
        fixed_motion_drawing(&mut grid, cells, area, growth, PROGRESS_SCALE);
        grid.apply(TileAction::Add, growth);
        grid.settle_for_test();
    }
    fixed_motion_drawing(&mut grid, cells, area, growth, PROGRESS_SCALE);
    grid.apply(TileAction::Remove, growth);
    let raw = CONTENT_CHANGE_TEST_PROGRESS[0];
    fixed_motion_drawing(&mut grid, cells, area, growth, raw);
    grid.retain_measured_destination_for_test();
    fixed_motion_drawing(&mut grid, cells, area, growth, raw);
    assert!(grid.has_retargeted_destination_for_test());
    grid
}

fn assert_width_retarget_tiles_bands(narrowing: bool) {
    let area = COLUMN_FOOT_TEST_AREA;
    let mut cells = WideningSummaryCells {
        content: ChangingSummaryCells {
            entries: CONTENT_CHANGE_TEST_ENTRIES[0],
            groups:  CONTENT_CHANGE_TEST_GROUPS,
        },
        width:   if narrowing { area.width } else { 0 },
    };
    let growth = TileGrowth {
        initial_rows: CONTENT_CHANGE_TEST_COLUMN_ROWS,
        widen_summary: true,
        ..changing_summary_growth()
    };
    let mut grid = cached_widening_summary_grid(&cells, growth);
    let clock = grid.motion_timing_for_test();
    cells.width = if narrowing { 0 } else { area.width };
    let mut last = Vec::new();
    for raw in std::iter::once(CONTENT_CHANGE_TEST_PROGRESS[0]).chain(
        (0..=SLIDING_TEST_INTERVALS)
            .map(|sample| sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS)
            .filter(|&progress| progress > CONTENT_CHANGE_TEST_PROGRESS[0]),
    ) {
        let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
        assert_eq!(clock, grid.motion_timing_for_test());
        assert_drawing_tiles_bands(&drawing, raw);
        assert_no_unusable_row(&cells, &drawing, 0, raw);
        let mut buffer = Buffer::empty(area);
        let widths = measurement_widths(&grid, area, growth, GridDisplay::Cells);
        draw_placements(
            &mut buffer,
            area,
            &drawing,
            &TileMeasurements {
                demands: &cells.demands(&widths),
                widths:  &widths,
            },
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );
        assert!(
            (area.top()..area.bottom())
                .any(|y| buffer_line(&buffer, y).contains(CONTENT_CHANGE_TEST_ROW))
        );
        last = column_geometry(&drawing);
    }
    grid.finish_motion_for_test();
    let settled = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    assert_drawing_tiles_bands(&settled, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert!(!grid.tick());
}

/// A key uses intermediate heights while its unchanged cells move one way.
fn assert_key_slides_column(
    grid: &mut TileGrid<u32>,
    cells: &impl TileCells<u32>,
    growth: TileGrowth,
    action: TileAction,
) {
    let area = COLUMN_FOOT_TEST_AREA;
    let before = fixed_motion_drawing(grid, cells, area, growth, PROGRESS_SCALE);
    grid.apply(action, growth);
    fixed_motion_drawing(grid, cells, area, growth, 0);
    let after = grid.drawing_at(area, growth, PROGRESS_SCALE);
    assert_eq!(after.column_bands.len(), 1);
    let changing = if action == TileAction::Add {
        &after
    } else {
        &before
    };
    let foot = changing.pieces.last();
    assert!(foot.is_some(), "the changed column has a foot cell");
    let Some(foot) = foot else {
        return;
    };
    let full_span = foot.placement.frame.clip().height.saturating_sub(1);
    let mut previous = before
        .placements()
        .map(|placement| (placement.content.clone(), placement.frame.clip().height))
        .collect::<Vec<_>>();
    let mut geometries = Vec::new();
    let mut spans = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        let drawing = fixed_motion_drawing(grid, cells, area, growth, raw);
        let geometry = column_geometry(&drawing);
        if raw < PROGRESS_SCALE && geometries.last() != Some(&geometry) {
            geometries.push(geometry);
        }
        let span = drawing
            .placements()
            .find(|placement| placement.content == foot.placement.content)
            .map_or(0, |placement| {
                placement.frame.clip().height.saturating_sub(1)
            });
        spans.push(span);
        assert_key_frame_keeps_height_rules(cells, &before, &after, &drawing, &mut previous);
    }
    assert_key_has_visible_height_steps(
        action,
        full_span,
        &spans,
        geometries.len().saturating_sub(1),
    );
    grid.finish_motion_for_test();
    let settled = fixed_motion_drawing(grid, cells, area, growth, PROGRESS_SCALE);
    assert_eq!(
        column_geometry(&settled),
        column_geometry(&after),
        "one key finishes without another motion"
    );
}

/// A visible cell starts with one body row and uses several moving heights.
fn assert_key_has_visible_height_steps(
    action: TileAction,
    full_span: u16,
    spans: &[u16],
    changes: usize,
) {
    assert!(
        spans.iter().any(|&span| span > 0 && span < full_span),
        "{action:?} skips every intermediate height: {spans:?}"
    );
    assert!(
        spans.windows(2).all(|pair| if action == TileAction::Add {
            pair[0] <= pair[1]
        } else {
            pair[0] >= pair[1]
        }),
        "{action:?} reverses its changing cell: {spans:?}"
    );
    if action == TileAction::Add {
        assert_eq!(
            spans.iter().find(|&&span| span > 0),
            Some(&MIN_TILE_HEIGHT.saturating_sub(1)),
            "the first visible arrival has one body row"
        );
    }
    assert!(
        changes >= SLIDING_TEST_MIN_CHANGES,
        "{action:?} changes fewer than three times before finishing: {spans:?}"
    );
}

fn title_motion(area: Rect, before: &[u32], after: &[u32], growth: TileGrowth) -> TileGrid<u32> {
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&title_demands(before), growth);
    grid.settle_for_test();
    grid.sync(&title_demands(after), growth);
    grid
}

fn collapsed_title_motion(
    area: Rect,
    before: &[u32],
    after: &[u32],
    growth: TileGrowth,
) -> TileGrid<u32> {
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&title_demands(before), growth);
    grid.settle_for_test();
    grid.collapse_queued_changes_for_test();
    grid.sync(&title_demands(after), growth);
    grid
}

fn assert_titles_once(name: &str, grid: &TileGrid<u32>, area: Rect, growth: TileGrowth) {
    for raw in motion_snapshots() {
        let drawing = grid.drawing_at(area, growth, raw);
        let buffer = draw_motion_fixture_with_width(area, &drawing, 64);
        let screen = area_lines(&buffer, area).concat();
        let mut contents = drawing
            .placements()
            .filter(|placement| {
                let clip = placement.frame.clip().intersection(area);
                clip.width > 2 && clip.height >= 2
            })
            .map(|placement| placement.content.clone())
            .collect::<Vec<_>>();
        contents.sort_by_key(|content| match content {
            TileContent::Summary => 0,
            TileContent::Group(id) => *id,
            TileContent::Empty(id) => u32::try_from(*id).unwrap_or(u32::MAX),
        });
        contents.dedup();
        for content in contents {
            let title = match content {
                TileContent::Summary => "summary".to_string(),
                TileContent::Group(id) => group_title(id),
                TileContent::Empty(_) => continue,
            };
            assert_eq!(
                screen.matches(&title).count(),
                1,
                "{name} at raw {raw} has the wrong count for {title:?}; frames={:?}",
                drawing
                    .placements()
                    .filter(|placement| placement.content == content)
                    .map(|placement| placement.frame)
                    .collect::<Vec<_>>()
            );
        }
        for piece in drawing.placements().filter(|piece| {
            drawing
                .placements()
                .filter(|other| other.content == piece.content)
                .count()
                == 2
        }) {
            let TileContent::Group(id) = piece.content else {
                continue;
            };
            let bottoms = drawing
                .column_bands
                .iter()
                .find(|band| {
                    let clip = piece.frame.clip();
                    clip.x == band.x && clip.width == band.width
                })
                .map(|band| band.bottom().saturating_sub(1))
                .into_iter()
                .collect::<Vec<_>>();
            assert_eq!(
                bottoms.len(),
                1,
                "{name} at raw {raw} places the piece in one column band"
            );
            for bottom in bottoms {
                assert!(
                    !buffer_line(&buffer, bottom).contains(&group_title(id)),
                    "{name} at raw {raw} puts the crossing title on a column floor"
                );
            }
        }
    }
}

/// Every title is on screen once through the step from `before` to `after`.
fn assert_titles_once_through(name: &str, before: &[u32], after: &[u32], growth: TileGrowth) {
    let grid = title_motion(TITLE_AREA, before, after, growth);
    assert_titles_once(name, &grid, TITLE_AREA, growth);
}

/// Whether this piece has enough visible room for `draw_number`'s output.
fn number_has_room(frame: PaneFrame, number: usize) -> bool {
    let contents = content_area(frame.inner());
    let clip = frame_inner(frame.clip());
    let indent = u16::try_from(TILE_NUMBER_INDENT.len()).unwrap_or(u16::MAX);
    let width = u16::try_from(number.to_string().len()).unwrap_or(u16::MAX);
    let left = contents.left().saturating_add(indent);
    let right = left.saturating_add(width);
    let row = i64::from(contents.top()) + i64::from(frame.shift());
    !contents.is_empty()
        && right <= contents.right()
        && left >= clip.left()
        && right <= clip.right()
        && row >= i64::from(clip.top())
        && row < i64::from(clip.bottom())
}

/// Whether `number` is present at this piece's own number position and style.
fn piece_draws_number(buffer: &Buffer, frame: PaneFrame, number: usize) -> bool {
    if !number_has_room(frame, number) {
        return false;
    }
    let contents = content_area(frame.inner());
    let indent = u16::try_from(TILE_NUMBER_INDENT.len()).unwrap_or(u16::MAX);
    let left = contents.left().saturating_add(indent);
    let row =
        u16::try_from(i64::from(contents.top()) + i64::from(frame.shift())).unwrap_or_default();
    number
        .to_string()
        .chars()
        .enumerate()
        .all(|(offset, digit)| {
            let x = left.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            let cell = &buffer[(x, row)];
            let mut encoded = [0; 4];
            cell.symbol() == digit.encode_utf8(&mut encoded) && cell.fg == title_color()
        })
}

/// Every empty-cell number is drawn by exactly one piece through one motion.
fn assert_empty_numbers_once(name: &str, grid: &TileGrid<u32>, growth: TileGrowth) {
    let mut saw_crossing = false;
    for raw in motion_snapshots() {
        let mut drawing = grid.drawing_at(TITLE_AREA, growth, raw);
        for piece in &mut drawing.pieces {
            if let TileContent::Group(id) = piece.placement.content {
                piece.placement.content =
                    TileContent::Empty(usize::try_from(id).unwrap_or(usize::MAX));
            }
        }
        let buffer = draw_motion_fixture_with_width(TITLE_AREA, &drawing, 64);
        let mut numbers = drawing
            .pieces
            .iter()
            .filter_map(|piece| match piece.placement.content {
                TileContent::Empty(number) => Some(number),
                TileContent::Summary | TileContent::Group(_) => None,
            })
            .collect::<Vec<_>>();
        numbers.sort_unstable();
        numbers.dedup();
        for number in numbers {
            let pieces = drawing
                .pieces
                .iter()
                .filter(|piece| piece.placement.content == TileContent::Empty(number))
                .collect::<Vec<_>>();
            saw_crossing |= pieces.len() == 2;
            if !pieces
                .iter()
                .any(|piece| number_has_room(piece.placement.frame, number))
            {
                continue;
            }
            let copies = pieces
                .iter()
                .filter(|piece| piece_draws_number(&buffer, piece.placement.frame, number))
                .count();
            assert_eq!(
                copies,
                1,
                "{name} at raw {raw} draws {copies} copies of empty cell {number}; frames={:?}",
                pieces
                    .iter()
                    .map(|piece| piece.placement.frame)
                    .collect::<Vec<_>>()
            );
        }
    }
    assert!(saw_crossing, "{name} exercises a crossing empty cell");
}

/// Every empty-cell number is drawn once through the step from `before` to `after`.
fn assert_empty_numbers_once_through(
    name: &str,
    before: &[u32],
    after: &[u32],
    growth: TileGrowth,
) {
    let grid = title_motion(TITLE_AREA, before, after, growth);
    assert_empty_numbers_once(name, &grid, growth);
}

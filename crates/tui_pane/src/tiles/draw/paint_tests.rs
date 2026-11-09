use std::cell::RefCell;

use super::*;
use crate::ColorMode;
use crate::FRAME_POLL_MILLIS;
use crate::PaneFocusState;
use crate::TileAction;
use crate::TileDemand;
use crate::TileFill;
use crate::TileView;
use crate::ToastActivation;
use crate::ToastStyle;
use crate::pane_background;
use crate::tiles::constants::COLUMN_FOOT_TEST_AREA;
use crate::tiles::constants::COLUMN_FOOT_TEST_BODY_STEP;
use crate::tiles::constants::COLUMN_FOOT_TEST_GROUP_COUNT;
use crate::tiles::constants::COLUMN_FOOT_TEST_MIN_HEIGHT;
use crate::tiles::constants::COLUMN_FOOT_TEST_SUMMARY_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_EMPTY_CELLS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_ENTRIES;
use crate::tiles::constants::CONTENT_CHANGE_TEST_GROUPS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_PROGRESS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_SOURCE_SPANS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_SPARE_SPANS;
use crate::tiles::constants::LARGE_CONTENT_TEST_GROUPS;
use crate::tiles::constants::LARGE_CONTENT_TEST_HEIGHT;
use crate::tiles::constants::LARGE_CONTENT_TEST_INITIAL_ROWS;
use crate::tiles::constants::LARGE_CONTENT_TEST_MEASUREMENT_BOUND;
use crate::tiles::constants::LARGE_CONTENT_TEST_ROW_STEP;
use crate::tiles::constants::LARGE_CONTENT_TEST_ROWS;
use crate::tiles::constants::LARGE_CONTENT_TEST_WIDTHS;
use crate::tiles::constants::LENDING_TEST_SUMMARY_TITLE;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::PROGRESS_SCALE;
use crate::tiles::constants::SETTLED_SPARE_TEST_HEIGHTS;
use crate::tiles::constants::SLIDING_TEST_INITIAL_ROWS;
use crate::tiles::constants::SLIDING_TEST_INTERVALS;
use crate::tiles::constants::SLIDING_TEST_MIN_CHANGES;
use crate::tiles::constants::SUMMARY_DONOR_TEST_SPANS;
use crate::tiles::constants::SUMMARY_STEP_TEST_ENTRY_ROWS;
use crate::tiles::constants::SUMMARY_STEP_TEST_HEADER_ROWS;
use crate::tiles::constants::SUMMARY_STEP_TEST_OPTIONAL_HEIGHTS;
use crate::tiles::constants::SUMMARY_STEP_TEST_PIECES;
use crate::tiles::constants::SUMMARY_STEP_TEST_ROWS;
use crate::tiles::constants::TILE_ANIMATION_MILLIS;
use crate::tiles::draw::tests::*;
use crate::tiles::draw::*;

#[test]
fn hidden_summary_paint_and_polls_keep_the_same_empty_drawing() {
    let area = Rect::new(0, 0, 80, 24);
    let cells = StubCells::new([1]);
    let (mut grid, _) = settled_grid(area, &cells);
    grid.set_view(TileView::Summary);
    grid.settle_for_test();
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(Color::Magenta));
    let previous = buffer.clone();
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        STAYING_GROWTH,
        TileGridContents::Hidden,
        &cells,
        ColorMode::Colored,
    );
    assert_eq!(buffer, previous, "hidden summary leaves the body unchanged");
    assert_eq!(grid.painted_summary_frames(), [] as [Rect; 0]);
    assert!(!grid.tick(), "poll uses the same hidden drawing as paint");
}

#[test]
fn a_shifted_summary_returns_only_the_frame_its_buffer_paints()
-> Result<(), Box<dyn std::error::Error>> {
    struct ToastFrameApp {
        framework: crate::Framework<Self>,
    }
    impl crate::AppContext for ToastFrameApp {
        type AppPaneId = ();
        type ToastAction = crate::NoToastAction;
        fn framework(&self) -> &crate::Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut crate::Framework<Self> { &mut self.framework }
    }
    let area = Rect::new(0, 0, 64, 24);
    let base = Rect::new(0, 1, 64, 8);
    let clip = Rect::new(0, 15, 64, 8);
    let drawing = TileDrawing {
        pieces:       vec![tile_piece(
            TileContent::Summary,
            PaneFrame::shifted(base, 14, clip),
            PieceName::Shown,
        )],
        column_bands: vec![],
    };
    let cells = StubCells::new([]);
    let widths = [(TileContent::Summary, 62)];
    let mut buffer = Buffer::empty(area);
    let frames = draw_placements(
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
    assert_eq!(buffer[(0, 15)].symbol(), "┌");
    assert_ne!(buffer[(0, 1)].symbol(), "┌");
    assert_eq!(frames, vec![clip]);
    let mut toasts =
        crate::Toasts::<ToastFrameApp>::with_settings(crate::ToastSettings::from_table(
            &"[toasts]\nplacement = 'top_right'\nwidth = 24\n".parse()?,
        )?);
    toasts.push_persistent(
        "toast",
        "body",
        ToastStyle::Normal,
        ToastActivation::Informational,
        3,
    );
    let allowed = toasts.area_avoiding_frames(frame_inner(area), &frames);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(64, 24))?;
    terminal.draw(|frame| {
        *frame.buffer_mut() = buffer.clone();
        crate::Renderable::render(
            &mut toasts,
            frame,
            allowed,
            &crate::ToastsRenderCtx {
                now:              std::time::Instant::now() + std::time::Duration::from_secs(1),
                pane_focus_state: PaneFocusState::Inactive,
            },
        );
    })?;
    let card = toasts.hits()[0].card_rect;
    assert_eq!(terminal.backend().buffer()[(card.x, card.y)].symbol(), "┌");
    assert!(card.bottom() < clip.top());
    assert!(
        card.top() < base.bottom(),
        "the unshifted frame would exclude these rows"
    );
    for x in 0..64 {
        assert_eq!(terminal.backend().buffer()[(x, 21)], buffer[(x, 21)]);
    }
    let mut hidden = Buffer::empty(area);
    let frames = draw_placements(
        &mut hidden,
        area,
        &drawing,
        &TileMeasurements {
            demands: &cells.demands(&widths),
            widths:  &widths,
        },
        TileGridContents::Hidden,
        &cells,
        ColorMode::Colored,
    );
    assert_eq!(frames, []);
    Ok(())
}

#[test]
fn a_cell_cut_at_its_source_never_reads_whole() {
    let area = Rect::new(0, 0, 80, 20);
    for (extent, ink) in [
        (CellContentExtent::Measured, success_color()),
        (CellContentExtent::CutAtSource, error_color()),
    ] {
        let cells = SourceCutCells {
            cells: StubCells::new([]),
            extent,
        };
        let mut grid = TileGrid::new();
        let mut buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            TileGrowth::default(),
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );
        let y = area.bottom() - 2;
        let line: String = (0..area.width).map(|x| buffer[(x, y)].symbol()).collect();
        let actual = line.find("content rows: 1").and_then(|byte| {
            let byte = byte + "content rows: ".len();
            u16::try_from(line[..byte].chars().count())
                .ok()
                .map(|x| buffer[(x, y)].fg)
        });
        assert_eq!(actual, Some(ink));
    }
}

#[test]
fn a_large_cell_frame_measures_each_content_allocation_once() {
    for width in LARGE_CONTENT_TEST_WIDTHS {
        for rows in LARGE_CONTENT_TEST_ROWS {
            let area = Rect::new(0, 0, width, LARGE_CONTENT_TEST_HEIGHT);
            let cells = CountedRowCells {
                rows,
                maximum_height: content_area(frame_inner(area)).height,
                queries: RefCell::new(Vec::new()),
            };
            let mut grid = TileGrid::new();
            grid.set_view(TileView::Cells);
            let growth = TileGrowth {
                initial_rows: LARGE_CONTENT_TEST_INITIAL_ROWS,
                ..TileGrowth::default()
            };
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
            let display = std::thread::scope(|readers| readers.spawn(|| grid.display()).join());
            assert!(matches!(display, Ok(GridDisplay::Cells)));
            assert!(!cells.queries.borrow().is_empty());
        }
    }
}

#[test]
fn a_toast_width_summary_never_draws_compact_with_room_above_its_foot()
-> Result<(), Box<dyn std::error::Error>> {
    let area = Rect::new(0, 0, 64, 50);
    let growth = TileGrowth {
        initial_rows:  12,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let before = Toast64MotionCells {
        groups:       5,
        summary_rows: 13,
    };
    let after = Toast64MotionCells {
        groups:       6,
        summary_rows: 18,
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(6);
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
    grid.collapse_queued_changes_for_test();

    let mut mates_hide_rows = false;
    // Sample the runner's 8 ms FRAME_POLL_MILLIS stride across the default
    // 720 ms TILE_ANIMATION_MILLIS motion, including its first and last frame.
    let frame_stride = usize::try_from(FRAME_POLL_MILLIS)?;
    let frame_progress = (0..TILE_ANIMATION_MILLIS)
        .step_by(frame_stride)
        .map(|elapsed| u32::try_from(elapsed * u64::from(PROGRESS_SCALE) / TILE_ANIMATION_MILLIS))
        .chain([Ok(PROGRESS_SCALE)]);
    for raw in frame_progress {
        let raw = raw?;
        let drawing = fixed_motion_drawing(&mut grid, &after, area, growth, raw);
        let summary = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Summary);
        assert!(summary.is_some(), "the motion keeps the summary");
        let Some(summary) = summary else {
            continue;
        };
        mates_hide_rows |= drawing.placements().any(|placement| {
            matches!(placement.content, TileContent::Group(_))
                && content_area(placement.frame.inner()).height < 18
        });
        assert!(
            placement_fits(&after, summary),
            "summary leaves a row at {raw}"
        );
    }
    assert!(mates_hide_rows, "the motion hides rows from a command cell");
    Ok(())
}

#[test]
fn a_summary_hiding_rows_beside_cells_keeps_the_cells() {
    let area = Rect::new(0, 0, 77, 40);
    let min_tile_height = 14;
    let cells = StubCells::demanding(50, (1..=5).map(|id| (id, 1)))
        .with_rows_drawn(TileContent::Summary, 50);
    let (grid, buffer) = settled_grid_in_view(area, &cells, TileView::Cells, min_tile_height);

    assert_eq!(grid.display(), GridDisplay::Cells);
    assert!(buffer_text(&buffer).contains("group body"));
}

#[test]
fn summary_view_shows_the_summary_alone_while_cells_fit() {
    let area = Rect::new(0, 0, 200, 50);
    let cells = StubCells::new(1..=5);
    let (grid, buffer) = settled_grid_in_view(area, &cells, TileView::Summary, MIN_TILE_HEIGHT);

    assert_eq!(grid.display(), GridDisplay::SummaryAlone);
    assert!(!buffer_text(&buffer).contains("group body"));
}

#[test]
fn cells_view_shows_cells_whenever_they_fit_at_the_floor() {
    let area = Rect::new(0, 0, 77, 40);
    let cells = StubCells::demanding(50, (1..=5).map(|id| (id, 1)))
        .with_rows_drawn(TileContent::Summary, 50);
    let (grid, buffer) = settled_grid_in_view(area, &cells, TileView::Cells, MIN_TILE_HEIGHT);

    assert_eq!(grid.display(), GridDisplay::Cells);
    assert!(buffer_text(&buffer).contains("group body"));
}

#[test]
fn no_view_draws_a_cell_under_its_floor() {
    let growth = TileGrowth::default();
    let cells = StubCells::new([1, 2]);
    let large = Rect::new(0, 0, 100, 40);
    let narrow = Rect::new(0, 0, TEST_TILE_WIDTH - 1, 40);

    for view in [TileView::Summary, TileView::Cells] {
        let mut grid = TileGrid::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.set_view(view);
        let mut buffer = Buffer::empty(large);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            large,
            growth,
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );
        grid.settle_for_test();

        buffer = Buffer::empty(narrow);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            narrow,
            growth,
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );

        assert_eq!(grid.display(), GridDisplay::SummaryAlone, "{view:?}");
        assert!(!buffer_text(&buffer).contains("group body"), "{view:?}");
    }
}

#[test]
fn a_piece_in_flight_leaves_no_cell_unpainted() {
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 60, 12);
    let middle_column = Rect::new(20, 0, 21, 12);
    let last_column = Rect::new(40, 0, 20, 12);
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Summary,
                crate::PaneFrame::new(Rect::new(0, 0, 21, 12)),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(7),
                crate::PaneFrame::shifted(Rect::new(20, 0, 21, 7), -2, middle_column),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(7),
                crate::PaneFrame::shifted(Rect::new(40, 6, 20, 6), 3, last_column),
                PieceName::Shown,
            ),
        ],
        column_bands: vec![Rect::new(0, 0, 21, 12), middle_column, last_column],
    };
    let demands = TileDemands {
        summary:       2,
        summary_width: 0,
        groups:        vec![TileDemand { id: 7, rows: 2 }],
    };
    let widths = [(TileContent::Summary, 19), (TileContent::Group(7), 19)];
    let cells = StubCells::new([7]);
    let mut buffer = Buffer::empty(area);

    draw_placements(
        &mut buffer,
        area,
        &drawing,
        &TileMeasurements {
            demands: &demands,
            widths:  &widths,
        },
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            assert_ne!(
                buffer[(x, y)].bg,
                Color::Reset,
                "cell ({x}, {y}) is unpainted"
            );
        }
    }
}

#[test]
fn a_line_over_an_overlapped_cell_sits_on_the_screen_ground() {
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 20, 10);
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Group(7),
                crate::PaneFrame::shifted(Rect::new(0, 5, 20, 5), -2, area),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(8),
                crate::PaneFrame::new(Rect::new(0, 0, 20, 6)),
                PieceName::Shown,
            ),
        ],
        column_bands: vec![area],
    };
    let demands = TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        vec![TileDemand { id: 7, rows: 2 }, TileDemand { id: 8, rows: 2 }],
    };
    let widths = [(TileContent::Group(7), 18), (TileContent::Group(8), 18)];
    let cells = StubCells::new([7, 8]);
    let draw = |contents| {
        let mut buffer = Buffer::empty(area);
        draw_placements(
            &mut buffer,
            area,
            &drawing,
            &TileMeasurements {
                demands: &demands,
                widths:  &widths,
            },
            contents,
            &cells,
            ColorMode::Colored,
        );
        buffer
    };

    let shown = draw(TileGridContents::Shown);
    let ground = screen_ground().unwrap_or(Color::Reset);
    assert_ne!(
        ground,
        Color::Reset,
        "the opaque screen has a painted ground"
    );
    assert_lattice_background(&shown, area, ground);

    let hidden = draw(TileGridContents::Hidden);
    assert_lattice_background(&hidden, area, Color::Reset);
}

#[test]
fn an_arriving_piece_draws_no_line_until_it_has_a_row() {
    let area = Rect::new(0, 0, 6, 5);
    let rect = Rect::new(0, 0, 6, 3);
    let start_shift =
        i64::from(frame_inner(area).bottom()).saturating_sub(i64::from(frame_inner(rect).top()));
    let title = group_title(7);
    let mut saw_edge_only = false;
    let mut saw_body = false;

    for raw in 0..=PROGRESS_SCALE {
        let shift = start_shift
            .saturating_sub(start_shift.saturating_mul(i64::from(raw)) / i64::from(PROGRESS_SCALE));
        let frame = PaneFrame::shifted(rect, i32::try_from(shift).unwrap_or(i32::MAX), area);
        let drawing = TileDrawing {
            pieces:       vec![tile_piece(TileContent::Group(7), frame, PieceName::Shown)],
            column_bands: vec![area],
        };

        let buffer = draw_motion_fixture(area, &drawing);
        let has_body = has_visible_body_row(frame);
        saw_edge_only |= !has_body;
        saw_body |= has_body;
        assert_eq!(
            buffer_text(&buffer).contains(&title),
            has_body,
            "arrival title visibility at raw {raw}; frame={frame:?}"
        );
        assert_eq!(
            doubled_line_failure(&buffer, area, "arrival", raw),
            None,
            "frame={frame:?}"
        );
    }

    assert!(saw_edge_only, "the arrival begins before its body enters");
    assert!(saw_body, "the arrival ends with a visible body row");
}

#[test]
fn a_summary_losing_content_during_a_key_gives_its_spare_rows_at_once() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[1].saturating_add(1),
        groups:  CONTENT_CHANGE_TEST_GROUPS.saturating_add(1),
    };
    let mut grid = changing_summary_grid(cells, 0);
    grid.record_settled_spans_for_test(&SUMMARY_DONOR_TEST_SPANS);
    grid.apply(TileAction::Add, changing_summary_growth());
    assert_motion_content_is_filled(&mut grid, cells, CONTENT_CHANGE_TEST_PROGRESS[0]);
    cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
    let clock = grid.motion_timing_for_test();
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw < CONTENT_CHANGE_TEST_PROGRESS[0] {
            continue;
        }
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        let endpoint = grid.drawing_at(
            COLUMN_FOOT_TEST_AREA,
            changing_summary_growth(),
            PROGRESS_SCALE,
        );
        assert_no_givable_spare_row(&cells, &drawing, &endpoint, raw);
        assert_eq!(clock, grid.motion_timing_for_test());
        last = column_geometry(&drawing);
    }
    grid.finish_motion_for_test();
    let settled = assert_motion_content_is_filled(&mut grid, cells, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert_no_givable_spare_row(&cells, &settled, &settled, PROGRESS_SCALE);
    assert!(!grid.tick());
}

#[test]
fn a_key_claimed_by_a_waiting_command_brings_no_second_empty_piece() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  1,
    };
    let growth = changing_summary_growth();
    let area = COLUMN_FOOT_TEST_AREA;
    let mut grid = changing_summary_grid(cells, 0);
    cells.groups = cells.groups.saturating_add(1);
    fixed_motion_drawing(
        &mut grid,
        &cells,
        area,
        growth,
        CONTENT_CHANGE_TEST_PROGRESS[0],
    );
    cells.groups = cells.groups.saturating_add(1);
    let widths = measurement_widths(&grid, area, growth, GridDisplay::Cells);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, growth);
    grid.apply(TileAction::Add, growth);
    let clock = grid.motion_timing_for_test();
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        assert_eq!(clock, grid.motion_timing_for_test());
        assert_drawing_tiles_bands(&drawing, raw);
        last = column_geometry(&drawing);
    }
    grid.finish_motion_for_test();
    let settled = assert_motion_content_is_filled(&mut grid, cells, PROGRESS_SCALE);
    assert_eq!(last, column_geometry(&settled));
    assert!(
        !grid.tick(),
        "a claimed key leaves no second opening behind it"
    );
    assert_eq!(
        settled.placements().count(),
        usize::try_from(cells.groups).unwrap_or_default() + 1
    );
}

#[test]
fn no_cell_cuts_a_row_while_a_mate_holds_a_spare_row_it_could_give() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, 0);
    grid.record_settled_spans_for_test(&CONTENT_CHANGE_TEST_SPARE_SPANS);
    grid.apply(TileAction::Add, changing_summary_growth());
    cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[0] {
            cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
        }
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        let endpoint = grid.drawing_at(
            COLUMN_FOOT_TEST_AREA,
            changing_summary_growth(),
            PROGRESS_SCALE,
        );
        assert_no_givable_spare_row(&cells, &drawing, &endpoint, raw);
    }
    grid.finish_motion_for_test();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        let endpoint = grid.drawing_at(
            COLUMN_FOOT_TEST_AREA,
            changing_summary_growth(),
            PROGRESS_SCALE,
        );
        assert_no_givable_spare_row(&cells, &drawing, &endpoint, raw);
    }
}

#[test]
fn an_arriving_cell_keeps_advancing_when_content_retargets_its_key() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let mut grid = changing_summary_grid(cells, 0);
    grid.apply(TileAction::Add, changing_summary_growth());
    let mut spans = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[0] {
            cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
        }
        let drawing = assert_motion_content_is_filled(&mut grid, cells, raw);
        if raw < PROGRESS_SCALE {
            let span = drawing
                .placements()
                .find(|piece| matches!(piece.content, TileContent::Empty(_)))
                .map_or(0, |piece| piece.frame.clip().height.saturating_sub(1));
            if span > 0 && spans.last() != Some(&span) {
                spans.push(span);
            }
        }
    }
    assert!(
        spans.len() >= SLIDING_TEST_MIN_CHANGES,
        "the arrival advances through filled heights: {spans:?}"
    );
    assert!(spans.windows(2).all(|pair| pair[0] < pair[1]));
    grid.finish_motion_for_test();
    assert!(!grid.tick(), "the arrival ends within the key's motion");
}

#[test]
fn a_resize_ends_a_content_retarget_for_good() {
    let mut cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let growth = changing_summary_growth();
    let mut grid = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS);
    grid.record_settled_spans_for_test(&CONTENT_CHANGE_TEST_SOURCE_SPANS);
    grid.apply(TileAction::Remove, growth);
    for sample in 0..SLIDING_TEST_INTERVALS {
        let raw = sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS;
        if raw >= CONTENT_CHANGE_TEST_PROGRESS[2] {
            cells.entries = CONTENT_CHANGE_TEST_ENTRIES[1];
        }
        assert_motion_content_is_filled(&mut grid, cells, raw);
    }
    let area = COLUMN_FOOT_TEST_AREA;
    let resized = Rect {
        width: TEST_TILE_WIDTH.saturating_mul(2),
        ..area
    };
    grid.set_draw_layout(resized, growth);
    fixed_motion_drawing(&mut grid, &cells, resized, growth, PROGRESS_SCALE);
    assert!(!grid.tick());
    grid.set_draw_layout(area, growth);
    let returned = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    let mut fresh = changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS.saturating_sub(1));
    let expected = fixed_motion_drawing(&mut fresh, &cells, area, growth, PROGRESS_SCALE);
    for piece in expected
        .placements()
        .filter(|piece| !matches!(piece.content, TileContent::Empty(_)))
    {
        assert_eq!(
            returned
                .placements()
                .find(|other| other.content == piece.content)
                .map(|other| other.frame.clip()),
            Some(piece.frame.clip())
        );
    }
    assert!(!grid.tick());
}

#[test]
fn a_settled_spare_row_check_rejects_a_donor_beside_a_cutting_mate() {
    let cells = ChangingSummaryCells {
        entries: CONTENT_CHANGE_TEST_ENTRIES[0],
        groups:  CONTENT_CHANGE_TEST_GROUPS,
    };
    let [donor_height, recipient_height] = SETTLED_SPARE_TEST_HEIGHTS;
    let area = COLUMN_FOOT_TEST_AREA;
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Group(1),
                PaneFrame::new(Rect {
                    height: donor_height,
                    ..area
                }),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(2),
                PaneFrame::new(Rect {
                    y: donor_height - 1,
                    height: recipient_height,
                    ..area
                }),
                PieceName::Shown,
            ),
        ],
        column_bands: vec![area],
    };
    let rejected = std::panic::catch_unwind(|| {
        assert_no_givable_spare_row(&cells, &drawing, &drawing, PROGRESS_SCALE);
    });
    assert!(
        rejected.is_err(),
        "settled spare rows receive no take-back exemption"
    );
}

#[test]
fn no_piece_rests_on_a_row_it_cannot_fill_while_a_mate_cuts_content() {
    let cells = PairedEntrySummaryCells;
    let area = COLUMN_FOOT_TEST_AREA;
    let growth = TileGrowth {
        initial_rows:  SLIDING_TEST_INITIAL_ROWS,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
    grid.settle_for_test();
    for key in SUMMARY_STEP_TEST_ROWS.len()..SUMMARY_STEP_TEST_PIECES {
        let before = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
        assert_no_unusable_row(&cells, &before, key, PROGRESS_SCALE);
        grid.apply(TileAction::Add, growth);
        fixed_motion_drawing(&mut grid, &cells, area, growth, 0);
        let after = grid.drawing_at(area, growth, PROGRESS_SCALE);
        let mut previous = before
            .placements()
            .map(|placement| (placement.content.clone(), placement.frame.clip().height))
            .collect::<Vec<_>>();
        let mut last_geometry = Vec::new();
        for raw in (0..SLIDING_TEST_INTERVALS)
            .map(|sample| sample * PROGRESS_SCALE / SLIDING_TEST_INTERVALS)
            .chain([PROGRESS_SCALE.saturating_sub(1), PROGRESS_SCALE])
        {
            let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
            assert_no_unusable_row(&cells, &drawing, key, raw);
            assert_key_frame_keeps_height_rules(&cells, &before, &after, &drawing, &mut previous);
            last_geometry = column_geometry(&drawing);
        }
        grid.finish_motion_for_test();
        let settled = fixed_motion_drawing(&mut grid, &cells, area, growth, PROGRESS_SCALE);
        assert_eq!(settled.pieces.len(), key.saturating_add(1));
        assert_eq!(settled.column_bands.len(), 1);
        assert_no_unusable_row(&cells, &settled, key, PROGRESS_SCALE);
        assert_eq!(
            last_geometry,
            column_geometry(&settled),
            "key {key} ends where it rests"
        );
    }
}

#[test]
fn startup_cells_leave_no_line_before_their_first_row() {
    let growth = TileGrowth::default();
    let cells = ColumnFootCells;
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);

    for _ in 0..COLUMN_FOOT_TEST_GROUP_COUNT {
        assert_column_foot_has_no_doubled_line(&mut grid, &cells, growth);
        grid.finish_motion_for_test();
    }
    assert_eq!(
        u32::try_from(
            grid.drawing_at(COLUMN_FOOT_TEST_AREA, growth, PROGRESS_SCALE)
                .pieces
                .len()
        ),
        Ok(COLUMN_FOOT_TEST_GROUP_COUNT + 1),
        "startup draws every admitted cell"
    );
}

/// An entering tile may extend below the terminal while its title is already visible.
#[test]
fn an_entering_tile_keeps_its_title_on_the_last_terminal_row() {
    let area = Rect::new(0, 0, 18, 5);
    let drawing = TileDrawing {
        pieces:       vec![tile_piece(
            TileContent::Group(7),
            PaneFrame::shifted(Rect::new(0, 0, 18, 4), 4, Rect::new(0, 4, 18, 6)),
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
            widths:  &[(TileContent::Group(7), 16)],
        },
        TileGridContents::Shown,
        &SegmentedTitleCells,
        ColorMode::Colored,
    );

    assert!(buffer_line(&buffer, 4).contains("worker · impl"));
    assert_eq!(buffer[(1, 4)].fg, Color::Blue);
    assert_eq!(buffer[(8, 4)].fg, Color::Yellow);
    assert_eq!(buffer_text(&buffer).matches("worker · impl").count(), 1);
}

#[test]
fn content_glyphs_do_not_cover_ground_only_rows() {
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 12, 9);
    let drawing = TileDrawing {
        pieces:       vec![
            tile_piece(
                TileContent::Group(7),
                PaneFrame::new(Rect::new(0, 0, 12, 3)),
                PieceName::Shown,
            ),
            tile_piece(
                TileContent::Group(8),
                PaneFrame::new(Rect::new(0, 6, 12, 3)),
                PieceName::Shown,
            ),
        ],
        column_bands: vec![area],
    };
    let ground = screen_ground().unwrap_or(Color::Reset);
    assert_ne!(ground, Color::Reset, "the opaque screen has a ground");
    let pane = pane_background(false);
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(ground));
    let target = 4;
    for y in frame_inner(area).top()..frame_inner(area).bottom() {
        if y != target {
            buffer[(area.left() + 1, y)].set_bg(pane);
        }
    }
    buffer[(area.left() + 2, target)].set_symbol("x");

    for failure in [
        row_gap_failure(&buffer, &drawing, "fixture", 0),
        bare_band_row_failure(&buffer, &drawing, "fixture", 0),
    ] {
        assert!(
            failure.is_some_and(|failure| failure.contains("row 4")),
            "the content glyph must not hide the ground-only row"
        );
    }
}

#[test]
fn a_collapsing_cell_leaves_no_ground_only_row_between_neighbours() {
    crate::set_transparent_background(false);
    crate::set_focused_pane_tint(true);
    let area = Rect::new(0, 0, 60, 18);
    let mut failures = Vec::new();
    for (motion, grid, growth) in [
        {
            let (grid, growth) = column_transition(area, true);
            ("middle arrival", grid, growth)
        },
        {
            let (grid, growth) = middle_departure(area);
            ("middle departure", grid, growth)
        },
    ] {
        for raw in motion_snapshots() {
            let drawing = grid.drawing_at(area, growth, raw);
            let buffer = draw_motion_fixture(area, &drawing);
            if let Some(failure) = row_gap_failure(&buffer, &drawing, motion, raw) {
                let frames: Vec<_> = drawing
                    .placements()
                    .map(|placement| (&placement.content, placement.frame))
                    .collect();
                failures.push(format!("{failure}; frames={frames:?}"));
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_grid_too_wide_for_its_window_draws_the_summary_alone() {
    let growth = TileGrowth {
        initial_rows:  1,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let cells = StubCells::new(1..=8);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let mut wide = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut wide,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();

    // No core-width tile fits below the configured tile floor.
    let area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TITLE_AREA.height);
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

    let text = buffer_text(&buffer);
    assert!(buffer_line(&buffer, 0).contains("summary"));
    assert!(text.contains("table body"));
    assert!(!text.contains("group body"));
    for y in 1..area.bottom() - 1 {
        for x in 1..area.right() - 1 {
            assert!(
                !is_lattice_glyph(buffer[(x, y)].symbol()),
                "summary-only frame has an interior divider at ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_widened_summary_has_no_interior_band_divider() {
    let growth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: true,
    };
    let mut cells = StubCells::new(1..=5);
    cells.demands.summary_width = 120;
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let mut buffer = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    buffer = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );

    let drawing = grid.drawing_at(TITLE_AREA, growth, PROGRESS_SCALE);
    assert_eq!(drawing.column_bands.len(), 2);
    let summary = drawing
        .placements()
        .find(|piece| piece.content == TileContent::Summary);
    assert!(summary.is_some(), "the summary is drawn");
    let Some(summary) = summary.map(|piece| piece.frame.clip()) else {
        return;
    };
    assert!(summary.width > drawing.column_bands[0].width);
    let inner = frame_inner(summary);
    for y in inner.top()..inner.bottom() {
        for x in inner.left()..inner.right() {
            assert!(
                !is_lattice_glyph(buffer[(x, y)].symbol()),
                "widened summary has an interior divider at ({x}, {y})"
            );
        }
    }
}

#[test]
fn the_cells_stand_settled_in_the_first_frame_that_holds_them() {
    let growth = TileGrowth {
        initial_rows:  1,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let cells = StubCells::new(1..=8);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let mut buffer = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    let narrow = Rect::new(0, 0, 100, TITLE_AREA.height);
    buffer = Buffer::empty(narrow);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        narrow,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );

    buffer = Buffer::empty(TITLE_AREA);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        TITLE_AREA,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    let start = grid.drawing_at(TITLE_AREA, growth, 0);
    let finish = grid.drawing_at(TITLE_AREA, growth, PROGRESS_SCALE);
    let frames = |drawing: TileDrawing<u32>| {
        drawing
            .pieces
            .into_iter()
            .map(|piece| {
                (
                    piece.placement.content,
                    piece.placement.frame.rect(),
                    piece.placement.frame.clip(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(frames(start), frames(finish));
    assert!(buffer_text(&buffer).contains("group body"));
}

#[test]
fn a_command_refused_in_a_small_window_opens_when_there_is_room() {
    let growth = TileGrowth::default();
    let cells = StubCells::new([1, 2]);
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let small = Rect::new(0, 0, 30, TITLE_AREA.height);
    let mut buffer = Buffer::empty(small);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        small,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    assert_eq!(grid.placements(small, growth).len(), 1);

    let large = Rect::new(0, 0, 100, TITLE_AREA.height);
    buffer = Buffer::empty(large);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        large,
        growth,
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    assert_eq!(grid.placements(large, growth).len(), 3);
}

#[test]
fn a_hidden_grid_too_small_draws_nothing() {
    let area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TITLE_AREA.height);
    let cells = StubCells::new([1]).with_summary_foot(SummaryFoot::Text(Line::raw(FOOT_TEXT)));
    let mut grid = TileGrid::new();
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        TileGrowth::default(),
        TileGridContents::Hidden,
        &cells,
        ColorMode::Colored,
    );

    assert_eq!(buffer_text(&buffer).trim(), "");
}

struct SourceCutCells {
    cells:  StubCells,
    extent: CellContentExtent,
}

impl TileCells<u32> for SourceCutCells {
    fn summary_title(&self) -> &str { self.cells.summary_title() }
    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        self.cells.demands(widths)
    }
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, ground: Color) {
        self.cells.draw(buffer, content, inner, ground);
    }
    fn content_extent(&self, _: &TileContent<u32>) -> CellContentExtent { self.extent }
}

#[derive(Debug, Eq, PartialEq)]
struct ContentAllocationQuery {
    content:  TileContent<u32>,
    width:    u16,
    height:   u16,
    retained: bool,
}

struct CountedRowCells {
    rows:           u16,
    maximum_height: u16,
    queries:        RefCell<Vec<ContentAllocationQuery>>,
}

impl CountedRowCells {
    fn measure(&self, content: &TileContent<u32>, area: Rect, retained: bool) -> u16 {
        assert!(
            area.height == u16::MAX || area.height <= self.maximum_height,
            "measurement exceeds the frame's content height: {} > {}",
            area.height,
            self.maximum_height
        );
        let mut queries = self.queries.borrow_mut();
        let key = ContentAllocationQuery {
            content: content.clone(),
            width: area.width,
            height: area.height,
            retained,
        };
        assert!(
            !queries.contains(&key),
            "repeated content allocation: {key:?}"
        );
        queries.push(key);
        let count = queries
            .iter()
            .filter(|query| &query.content == content)
            .count();
        assert!(
            count <= LARGE_CONTENT_TEST_MEASUREMENT_BOUND,
            "content exceeds the frame measurement bound: {content:?}"
        );
        match content {
            TileContent::Summary => {
                area.height.min(self.rows) / LARGE_CONTENT_TEST_ROW_STEP
                    * LARGE_CONTENT_TEST_ROW_STEP
            },
            TileContent::Group(_) => area.height.min(1),
            TileContent::Empty(_) => 0,
        }
    }
}

impl TileCells<u32> for CountedRowCells {
    fn summary_title(&self) -> &str { LENDING_TEST_SUMMARY_TITLE }
    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(self.rows),
            summary_width: 0,
            groups:        (1..=LARGE_CONTENT_TEST_GROUPS)
                .map(|id| TileDemand {
                    id,
                    rows: usize::from(self.rows),
                })
                .collect(),
        }
    }
    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        self.measure(content, area, false)
    }
    fn rows_kept(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        self.measure(content, area, true)
    }
    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}

fn assert_lattice_background(buffer: &Buffer, area: Rect, expected: Color) {
    let mut lines = 0;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &buffer[(x, y)];
            if is_lattice_glyph(cell.symbol()) {
                lines += 1;
                assert_eq!(cell.bg, expected, "line background at ({x}, {y})");
            }
        }
    }
    assert!(lines > 0, "the placements draw lattice lines");
}

/// One cell leaving between two residents in a single column.
fn middle_departure(area: Rect) -> (TileGrid<u32>, TileGrowth) {
    let growth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let groups = |ids: &[u32]| TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
    };
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&groups(&[7, 8, 9]), growth);
    grid.settle_for_test();
    grid.sync(&groups(&[7, 9]), growth);
    (grid, growth)
}

fn row_gap_failure(
    buffer: &Buffer,
    drawing: &TileDrawing<u32>,
    motion: &str,
    raw: u32,
) -> Option<String> {
    for &band in &drawing.column_bands {
        let bodies = drawing
            .placements()
            .filter(|placement| {
                let clip = placement.frame.clip();
                clip.left() == band.left() && clip.right() == band.right()
            })
            .filter_map(|placement| visible_body_rows(placement.frame))
            .collect::<Vec<_>>();
        if bodies.len() < 2 {
            continue;
        }
        let Some(first_body_row) = bodies.iter().map(|body| body.start).min() else {
            continue;
        };
        let Some(last_body_row) = bodies.iter().map(|body| body.end).max() else {
            continue;
        };
        let ground = screen_ground().unwrap_or(Color::Reset);
        for y in first_body_row..last_body_row {
            let interior = band.left().saturating_add(1)..band.right().saturating_sub(1);
            if !row_has_coverage(buffer, drawing, ground, interior, y) {
                return Some(format!(
                    "{motion} at raw {raw} leaves row {y} bare between neighbouring cells"
                ));
            }
        }
    }
    None
}

struct ColumnFootCells;

impl TileCells<u32> for ColumnFootCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(COLUMN_FOOT_TEST_SUMMARY_ROWS),
            summary_width: 0,
            groups:        (1..=COLUMN_FOOT_TEST_GROUP_COUNT)
                .map(|id| TileDemand {
                    id,
                    rows: usize::from(COLUMN_FOOT_TEST_SUMMARY_ROWS),
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Empty(_) => 0,
            TileContent::Group(_) if area.height < COLUMN_FOOT_TEST_BODY_STEP => 0,
            TileContent::Summary | TileContent::Group(_) => {
                area.height.min(COLUMN_FOOT_TEST_SUMMARY_ROWS)
            },
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

/// A summary with complete two-row entries beside three cut commands.
struct PairedEntrySummaryCells;

impl TileCells<u32> for PairedEntrySummaryCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(SUMMARY_STEP_TEST_ROWS[0]),
            summary_width: 0,
            groups:        SUMMARY_STEP_TEST_ROWS
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
        match content {
            TileContent::Summary if area.height < SUMMARY_STEP_TEST_ROWS[0] => {
                let header = area.height.min(SUMMARY_STEP_TEST_HEADER_ROWS);
                header.saturating_add(
                    area.height.saturating_sub(header) / SUMMARY_STEP_TEST_ENTRY_ROWS
                        * SUMMARY_STEP_TEST_ENTRY_ROWS,
                )
            },
            TileContent::Summary => SUMMARY_STEP_TEST_ROWS[0],
            TileContent::Group(id) => SUMMARY_STEP_TEST_ROWS
                .get(usize::try_from(*id).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or_default()
                .min(area.height),
            TileContent::Empty(_) => 0,
        }
    }

    fn rows_kept(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        let drawn = self.rows_drawn(content, area);
        if let TileContent::Group(id) = content
            && SUMMARY_STEP_TEST_OPTIONAL_HEIGHTS
                .get(usize::try_from(id.saturating_sub(1)).unwrap_or(usize::MAX))
                == Some(&area.height)
        {
            drawn.saturating_sub(1)
        } else {
            drawn
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

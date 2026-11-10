use super::*;

#[test]
fn idle_visual_motion_bands_and_title_transfers_need_one_body_paint() {
    let mut grid = opening_column();
    let area = test_area();
    let growth = redistribute(4);
    let cells = MotionPaintCells(grid.demands.clone());
    let widths = grid.content_widths(area, growth);
    let mut last_buffer = Buffer::empty(area);
    let mut paints = 0;
    let mut band_changes = 0;
    let mut title_changes = 0;
    for raw in motion_steps() {
        let drawing = grid.drawing_at(area, growth, raw);
        if let LastPainted::Painted { visual, .. } = &grid.last_painted {
            band_changes += usize::from(drawing.column_bands != visual.drawing.column_bands);
            title_changes += usize::from(
                drawing
                    .pieces
                    .iter()
                    .map(|piece| piece.name)
                    .collect::<Vec<_>>()
                    != visual
                        .drawing
                        .pieces
                        .iter()
                        .map(|piece| piece.name)
                        .collect::<Vec<_>>(),
            );
        }
        let due = grid.visual_repaint_at(raw);
        let mut buffer = Buffer::empty(area);
        let frames = draw::draw_placements(
            &mut buffer,
            area,
            &drawing,
            &TileMeasurements {
                demands: &cells.0,
                widths:  &widths,
            },
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );
        if buffer != last_buffer {
            assert_eq!(due, crate::Repaint::Needed, "visible motion at {raw}");
            paints += 1;
        }
        grid.record_painted_body(drawing, TileGridContents::Shown, frames);
        assert_eq!(
            grid.visual_repaint_at(raw),
            crate::Repaint::NotNeeded,
            "paint commits {raw}"
        );
        last_buffer = buffer;
    }
    assert!(paints > 1, "motion draws several distinct frames");
    assert!(band_changes > 0, "columns move in this fixture");
    assert!(title_changes > 0, "title ownership changes in this fixture");
}

/// A frozen transition has one drawn geometry across many polls.
#[test]
fn idle_visual_equal_motion_polls_do_not_consume_the_painted_frame() {
    let (mut grid, area, growth, demands) = demand_resize_motion_fixture();
    if let GridMotion::Moving(transition) = &mut grid.motion {
        transition.started = Instant::now() + std::time::Duration::from_secs(60);
    }
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    crate::draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        &MotionPaintCells(demands),
        ColorMode::Colored,
    );
    let mut frames = 0;
    for _ in 0..32 {
        frames += usize::from(grid.tick());
    }
    assert_eq!(frames, 0, "equal motion geometry asks for no body frame");
    if let GridMotion::Moving(transition) = &mut grid.motion {
        transition.started = Instant::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .expect("test clock can precede the completed motion");
    }
    assert!(grid.tick(), "the final geometry still needs painting");
    assert!(grid.tick(), "a poll cannot consume an unpainted change");
}

#[test]
fn an_action_growth_change_replans_the_motion_endpoints() {
    let (mut grid, area, _, _) = demand_resize_motion_fixture();
    let growth = redistribute(1);
    grid.apply(TileAction::FocusDown, growth);
    assert_growth_keeps_groups_visible(&grid, area, growth);
}

#[test]
fn unchanged_motion_frames_reuse_endpoint_admission() {
    let (mut grid, area, growth, demands) = demand_resize_motion_fixture();
    assert!(grid.holds_in(area, growth));
    let checks = grid.admission_checks.load(AtomicOrdering::Relaxed);
    for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
        grid.sync_rendered_demands(&demands, growth);
        grid.set_layout(area, growth);
        assert_eq!(grid.display(), GridDisplay::Cells);
        assert_growth_keeps_groups_visible(&grid, area, growth);
        grid.measure_row_steps(
            area,
            growth,
            progress,
            |_, area| area.height,
            |_, area| area.height,
        );
    }
    assert_eq!(
        grid.admission_checks.load(AtomicOrdering::Relaxed),
        checks,
        "unchanged frames must not validate admission again"
    );
    grid.set_min_tile_height(6);
    assert!(grid.holds_in(area, growth));
    assert_eq!(
        grid.admission_checks.load(AtomicOrdering::Relaxed),
        checks + 1,
        "changed rules need one new admission check"
    );
}

#[test]
fn an_area_change_discards_queued_endpoints_and_measured_motion_heights() {
    assert_layout_change_discards_old_motion_geometry(MotionLayoutChange::GridArea);
}

#[test]
fn a_freed_row_goes_to_a_mate_whose_next_step_it_completes() {
    let mut settled = [6, 6, 6, 5, 6];
    let exact = [17, 11, 15, 15, 12];
    let minimum = [5, 5, 5, 5, 6];

    let mut unused = measured_column_unused;
    let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
    serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

    assert_eq!(settled, [7, 6, 5, 5, 6]);
}

#[test]
fn a_settled_grid_places_one_piece_per_cell() {
    let placements = TileGrid::<u32>::new().placements(test_area(), redistribute(4));
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].content, TileContent::Summary);
    assert_eq!(placements[0].frame.rect(), test_area());
}

#[test]
fn no_unchanged_cell_reverses_height_across_one_key() {
    let (mut grid, area, growth) = depth_changing_key_grid();
    assert_key_keeps_unchanged_heights_one_way(&mut grid, area, growth, TileAction::Add);
    assert_key_keeps_unchanged_heights_one_way(&mut grid, area, growth, TileAction::Remove);
}

#[test]
fn filled_motion_sources_begin_at_their_settled_heights() {
    for (before, after) in [
        (&[6, 6, 9][..], &[8, 7, 6][..]),
        (&[6, 19, 9][..], &[19, 8, 7][..]),
        (&[6, 6, 10][..], &[7, 8, 7][..]),
    ] {
        let (mut grid, area, growth) = exact_height_motion(before, after);
        let drawing = filled_height_drawing(&mut grid, area, growth, 0);

        for (piece, &settled_height) in drawing.pieces.iter().zip(before) {
            assert_eq!(piece.placement.frame.clip().height, settled_height);
            assert_eq!(
                piece.height_snap,
                PieceHeightSnap::Search {
                    start: settled_height,
                    end:   match piece.height_snap {
                        PieceHeightSnap::Search { end, .. } => end,
                        PieceHeightSnap::Settled | PieceHeightSnap::KeepStartingHeight =>
                            settled_height,
                    },
                }
            );
        }
    }
}

#[test]
fn mixed_height_changes_move_each_piece_one_way() {
    assert_piece_heights_move_one_way(&[6, 6, 9], &[8, 7, 6]);
}

#[test]
fn a_large_height_shrink_moves_one_way() {
    assert_piece_heights_move_one_way(&[6, 19, 9], &[19, 8, 7]);
}

#[test]
fn mixed_snapped_height_changes_move_one_way() {
    assert_snapped_heights_move_one_way(&[6, 6, 9], &[8, 7, 6], &[&[6, 8], &[6, 7], &[6, 7, 9]]);
}

#[test]
fn selected_piece_heights_are_not_reversed_by_absolute_divider_limits() {
    let (mut grid, area, growth) = exact_height_motion(&[8, 8, 8, 8], &[10, 6, 10, 6]);
    let raw = PROGRESS_SCALE / 2;
    let selected = vec![10, 8, 8, 6];
    let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
    let mut row_usage = Vec::new();
    for (piece, &height) in unsnapped.pieces.iter().zip(&selected) {
        let rect = Rect {
            height,
            ..piece.placement.frame.clip()
        };
        let content_area = draw::content_area(frame_inner(rect));
        measure_frame_row_usage(
            &mut row_usage,
            &piece.placement.content,
            content_area,
            &mut |_, area| area.height,
        );
    }
    grid.frame_row_usage = row_usage;
    grid.motion_heights = MotionHeightPath {
        area,
        growth,
        progress: raw,
        piece_heights: selected.clone(),
    };

    let actual = grid
        .drawing_at(area, growth, raw)
        .placements()
        .map(|placement| placement.frame.clip().height)
        .collect::<Vec<_>>();

    assert_eq!(actual, selected);
}

#[test]
fn a_shrinking_piece_skips_the_height_that_leaves_a_row_unused() {
    let (mut grid, area, growth) = exact_height_motion(&[11, 11, 11, 13], &[15, 7, 15, 9]);
    let raw = PROGRESS_SCALE * 3 / 4;
    let selected = vec![15, 11, 11, 9];
    let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
    let mut row_usage = Vec::new();
    for (piece_index, piece) in unsnapped.pieces.iter().enumerate() {
        for height in 9..=13 {
            let rect = Rect {
                height,
                ..piece.placement.frame.clip()
            };
            let content_area = draw::content_area(frame_inner(rect));
            measure_frame_row_usage(
                &mut row_usage,
                &piece.placement.content,
                content_area,
                &mut |_, area| {
                    if piece_index == 3 && height == 10 {
                        area.height.saturating_sub(1)
                    } else {
                        area.height
                    }
                },
            );
        }
    }
    grid.frame_row_usage = row_usage;
    grid.motion_heights = MotionHeightPath {
        area,
        growth,
        progress: raw,
        piece_heights: selected,
    };

    let drawing = grid.drawing_at(area, growth, raw);
    let shrinking = &drawing.pieces[3];
    let rect = shrinking.placement.frame.clip();
    let content_area = draw::content_area(frame_inner(rect));

    assert_eq!(rect.height, 9);
    assert_ne!(
        frame_height_fit(
            &grid.frame_row_usage,
            &shrinking.placement.content,
            content_area,
        ),
        FrameHeightFit::LeavesGap
    );

    let (mut grid, area, growth) = exact_height_motion(&[11, 11, 11, 13], &[15, 7, 15, 9]);
    let mut heights = Vec::new();
    for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| {
                let framed = area
                    .height
                    .saturating_add(TILE_BORDER_ROWS)
                    .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                if content == &TileContent::Group(3) && framed == 10 {
                    area.height.saturating_sub(1)
                } else {
                    area.height
                }
            },
            |_, area| area.height,
        );
        let drawing = grid.drawing_at(area, growth, raw);
        let shrinking = drawing
            .placements()
            .find(|placement| placement.content == TileContent::Group(3))
            .expect("the shrinking resident remains in the drawing");
        let rect = shrinking.frame.clip();
        if raw < PROGRESS_SCALE {
            let content_area = draw::content_area(frame_inner(rect));
            assert_ne!(
                frame_height_fit(&grid.frame_row_usage, &shrinking.content, content_area,),
                FrameHeightFit::LeavesGap,
                "the shrinking resident leaves a gap at {raw}: {rect:?}"
            );
        }
        heights.push(rect.height);
    }
    assert!(heights.windows(2).all(|pair| pair[0] >= pair[1]));
    for expected in [13, 12, 11, 9] {
        assert!(heights.contains(&expected), "missing height {expected}");
    }
    assert!(!heights.contains(&10));
}

#[test]
fn returning_to_a_motion_source_area_keeps_the_resized_destination() {
    let source_area = test_area();
    let resized_area = Rect::new(3, 2, TEST_WIDTH - 6, TEST_HEIGHT - 4);
    let growth = redistribute(4);
    let mut grid = synced_motion(&[7, 8, 9], &[7, 9], growth);
    let partial = grid.drawing_at(source_area, growth, PROGRESS_SCALE / 2);
    assert!(
        partial
            .placements()
            .any(|placement| placement.content == TileContent::Group(8)),
        "the departed piece is still visible before the resize"
    );
    assert!(
        grid.progress() < PROGRESS_SCALE,
        "the clock is still running"
    );

    grid.set_draw_layout(resized_area, growth);
    for raw in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
        let drawing = grid.drawing_at(resized_area, growth, raw);
        assert!(
            drawing
                .placements()
                .all(|placement| placement.content != TileContent::Group(8)),
            "the departed piece returns in the resized area at {raw}"
        );
        assert_eq!(
            endpoint_rects(&drawing),
            settled_rects(
                &Grid::new(resized_area, &grid.drawn_held(), growth, &grid.settings),
                grid.count(),
            ),
            "resized destination at {raw}"
        );
    }

    grid.set_draw_layout(source_area, growth);
    for raw in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
        let drawing = grid.drawing_at(source_area, growth, raw);
        assert!(
            drawing
                .placements()
                .all(|placement| placement.content != TileContent::Group(8)),
            "the departed piece returns in the source area at {raw}"
        );
        assert_eq!(
            endpoint_rects(&drawing),
            settled_rects(
                &Grid::new(source_area, &grid.drawn_held(), growth, &grid.settings),
                grid.count(),
            ),
            "source-area destination at {raw}"
        );
    }
}

#[test]
fn an_every_other_height_shrink_moves_one_way() {
    assert_snapped_heights_move_one_way(
        &[6, 19, 9],
        &[19, 8, 7],
        &[
            &[6, 8, 10, 12, 14, 16, 18, 19],
            &[8, 10, 12, 14, 16, 18, 19],
            &[7, 9],
        ],
    );
}

#[test]
fn a_height_search_reaches_the_nearest_valid_height_each_way() {
    let (mut grid, area, growth) = exact_height_motion(&[19, 27], &[27, 19]);
    let raw = PROGRESS_SCALE / 2;
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |_, area| match area.height {
            0..=16 => area.height,
            17..=23 => 16,
            24.. => 24,
        },
        |_, area| area.height,
    );
    let heights = grid
        .drawing_at(area, growth, raw)
        .placements()
        .map(|placement| placement.frame.clip().height)
        .collect::<Vec<_>>();

    assert!(
        heights.iter().all(|height| [19, 27].contains(height)),
        "the midpoint uses only the nearest valid endpoint heights: {heights:?}"
    );
}

#[test]
fn a_painted_optional_row_fills_a_moving_height() {
    let content = TileContent::<u32>::Summary;
    let rect = Rect::new(0, 0, 40, 5);
    let area = draw::content_area(frame_inner(rect));
    let mut usage = Vec::new();
    measure_frame_row_usage(&mut usage, &content, area, &mut |_, _| 2);
    let piece = BandPiece {
        column:   0,
        motion:   BandPieceMotion::Resident {
            before: rect,
            after:  rect,
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

    assert_eq!(area.height, 2);
    assert_eq!(
        frame_height_fit(&usage, &piece.drawn.content, area),
        FrameHeightFit::Filled
    );
    assert!(
        HeightSnap::Frame {
            row_usage: &usage,
            path:      PieceHeightPath::AdvanceFrom(&[]),
        }
        .accepts(&piece, rect)
    );
}

#[test]
fn nonmonotone_starting_dividers_keep_every_piece_inside_its_band() {
    let band = Rect::new(43, 79, 40, 14);
    let rects = [
        Rect::new(43, 79, 40, 12),
        Rect::new(43, 85, 40, 2),
        Rect::new(43, 86, 40, 7),
    ];
    let pieces: Vec<_> = rects
        .iter()
        .enumerate()
        .map(|(index, &rect)| BandPiece {
            column:   0,
            motion:   BandPieceMotion::Resident {
                before: rect,
                after:  rect,
            },
            progress: 0,
            drawn:    CellAppearance {
                identity: CellIdentity::Slot(Slot::Group(index)),
                content:  TileContent::Group(index),
                focused:  false,
            },
        })
        .collect();
    let held = HeldCellLayout {
        rows: row_demands(&[4; 4]),
        ..HeldCellLayout::default()
    };
    let grid = Grid::new(band, &held, TileGrowth::default(), &TileSettings::default());
    let indices = [0, 1, 2];
    let search = BandHeightSearch {
        before_grid: &grid,
        after_grid: &grid,
        pieces: &pieces,
        in_band: &indices,
        band,
        min_tile_height: 1,
        height_snap: &HeightSnap::Off,
    };
    let dividers = band_piece_dividers(&search, band, band);
    for (position, piece) in pieces.iter().enumerate() {
        let rect = band_piece_rect(
            &grid,
            &grid,
            piece,
            band,
            dividers[position],
            dividers[position + 1],
        );
        let frame = piece_frame(piece.motion, rect, band);
        assert_eq!(rect.intersection(band), rect, "{dividers:?}: {rect:?}");
        assert_eq!(frame.clip().intersection(band), frame.clip());
    }
}

#[test]
fn a_departing_piece_with_no_content_can_collapse() {
    let content = TileContent::Group(1);
    let rect = Rect::new(0, 0, 40, 6);
    let area = draw::content_area(frame_inner(rect));
    let mut usage = Vec::new();
    measure_frame_row_usage(&mut usage, &content, area, &mut |_, _| 0);
    let piece = BandPiece {
        column:   0,
        motion:   BandPieceMotion::Leaving {
            before:  Rect::new(0, 0, 40, 12),
            edge:    BandEdge::Bottom,
            sliding: false,
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
        HeightSnap::Frame {
            row_usage: &usage,
            path:      PieceHeightPath::AdvanceFrom(&[]),
        }
        .accepts(&piece, rect)
    );
}

#[test]
fn a_moving_piece_snaps_to_a_height_its_content_fills() {
    let (mut grid, area, growth) = stepped_height_motion();
    let mut calls = 0;
    for raw in 0..=PROGRESS_SCALE {
        let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
        for placement in drawing.placements() {
            assert!(
                stepped_height_fills(&placement.content, placement.frame.clip()),
                "{:?} is not filled at {raw}",
                placement.frame.clip()
            );
        }
    }
}

#[test]
fn a_snapped_height_moves_one_way_through_a_motion() {
    let (mut grid, area, growth) = stepped_height_motion();
    let mut calls = 0;
    let contents = [
        TileContent::Summary,
        TileContent::Group(1),
        TileContent::Group(2),
    ];
    let mut heights = vec![Vec::new(); contents.len()];
    for raw in 0..=PROGRESS_SCALE {
        let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
        for (content, heights) in contents.iter().zip(&mut heights) {
            if let Some(placement) = drawing
                .placements()
                .find(|placement| &placement.content == content)
            {
                heights.push(placement.frame.clip().height);
            }
        }
    }
    for (content, heights) in contents.iter().zip(heights) {
        let Some((&start, &finish)) = heights.first().zip(heights.last()) else {
            continue;
        };
        let one_way = if start <= finish {
            heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(one_way, "{content:?} reverses through {heights:?}");
        let start_endpoint = grid
            .drawing_at(area, growth, 0)
            .placements()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.clip().height);
        let finish_endpoint = grid
            .drawing_at(area, growth, PROGRESS_SCALE)
            .placements()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.clip().height);
        if let (Some(start_endpoint), Some(finish_endpoint)) = (start_endpoint, finish_endpoint) {
            assert_eq!(start, start_endpoint);
            assert_eq!(finish, finish_endpoint);
        }
    }
}

#[test]
fn two_stepped_mates_both_stand_at_heights_they_fill() {
    let (mut grid, area, growth) = stepped_height_motion();
    let mut calls = 0;
    for raw in 0..=PROGRESS_SCALE {
        let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
        for content in [TileContent::Summary, TileContent::Group(1)] {
            let rect = drawing
                .placements()
                .find(|placement| placement.content == content)
                .map(|placement| placement.frame.clip())
                .expect("both resident mates remain in the band");
            assert!(
                rect.height >= MIN_TILE_HEIGHT,
                "{content:?} at {raw}: {rect:?}"
            );
            assert!(
                stepped_height_fills(&content, rect),
                "{content:?} at {raw}: {rect:?}"
            );
        }
    }
}

#[test]
fn a_seven_cell_moving_frame_limits_row_probes() {
    let area = Rect::new(0, 0, 200, 50);
    let growth = redistribute(12);
    let demands = |ids: RangeInclusive<u32>| TileDemands {
        summary:       100,
        summary_width: 0,
        groups:        ids.map(|id| TileDemand { id, rows: 100 }).collect(),
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(40);
    grid.set_min_tile_height(6);
    grid.set_layout(area, growth);
    grid.sync(&demands(1..=5), growth);
    grid.settle_for_test();
    grid.collapse_queued_changes_for_test();
    grid.sync(&demands(1..=6), growth);
    let endpoint_areas = [0, PROGRESS_SCALE]
        .into_iter()
        .flat_map(|raw| grid.drawing_at(area, growth, raw).pieces)
        .map(|piece| {
            let area = draw::content_area(frame_inner(piece.placement.frame.clip()));
            (piece.placement.content, area.width, area.height)
        })
        .collect::<Vec<_>>();
    let mut drawn_calls = 0_usize;
    let mut kept_calls = 0_usize;
    let raw = PROGRESS_SCALE / 2;
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |content, area| {
            drawn_calls += 1;
            if area.height == u16::MAX {
                return match content {
                    TileContent::Summary => MIN_TILE_HEIGHT
                        .saturating_sub(TILE_BORDER_ROWS)
                        .saturating_sub(TILE_ROWS_READOUT_HEIGHT),
                    TileContent::Group(_) | TileContent::Empty(_) => area.height,
                };
            }
            if endpoint_areas.iter().any(|(endpoint, width, height)| {
                endpoint == content && *width == area.width && *height == area.height
            }) {
                area.height
            } else {
                area.height.saturating_sub(1)
            }
        },
        |_, area| {
            kept_calls += 1;
            area.height
        },
    );
    let _ = grid.drawing_at(area, growth, raw);
    assert_eq!(
        drawn_calls.saturating_add(kept_calls),
        SEVEN_CELL_MOVING_ROW_PROBE_COUNT
    );
}

#[test]
fn motion_endpoints_keep_every_present_cell_in_its_settled_rect() {
    let one_column = redistribute(4);
    let middle_departure = synced_motion(&[7, 8, 9], &[7, 9], one_column);
    assert_settled_motion_endpoints("middle departure", &middle_departure, one_column);

    let mut middle_arrival = synced_motion(&[7, 9], &[7, 9], one_column);
    middle_arrival.add(one_column);
    middle_arrival.settle_for_test();
    middle_arrival.add(one_column);
    middle_arrival.settle_for_test();
    middle_arrival.sync(&quiet(&[7, 8, 9]), one_column);
    assert_settled_motion_endpoints("middle arrival", &middle_arrival, one_column);

    let end_departure = synced_motion(&[7, 8, 9], &[7, 8], one_column);
    assert_settled_motion_endpoints("end departure", &end_departure, one_column);
    let end_arrival = synced_motion(&[7, 8], &[7, 8, 9], one_column);
    assert_settled_motion_endpoints("end arrival", &end_arrival, one_column);

    let crossing_growth = redistribute(3);
    let crossing = synced_motion(&[7, 8], &[7, 8, 9], crossing_growth);
    assert_settled_motion_endpoints("column crossing", &crossing, crossing_growth);
}

/// The complaint this closing answers: a hole walking to the end of
/// the grid meant every step drew it travelling one way through the
/// cell travelling the other, two boxes crossing in the space one
/// was closing over. Mid-transition each survivor keeps one piece
/// and the departing cell has the one piece that collapses.
#[test]
fn one_departing_piece_is_in_flight_while_the_grid_closes() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[7, 9]), redistribute(4));
    let placements = grid.placements(test_area(), redistribute(4));
    assert_eq!(
        placements.len(),
        4,
        "the summary, two survivors and one departing command"
    );
}

/// The complaint this answers: two cells exchanging places were
/// drawn each travelling through the other, borders crossing where
/// the eye was following one box. Every step of a reorder now adds
/// or takes away a single cell, so whatever survives a step stands
/// in the order it already stood in.
#[test]
fn no_step_of_a_reorder_draws_two_cells_crossing() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7, 8, 9, 10]), redistribute(4));
    grid.settle_for_test();

    grid.sync(&quiet(&[9, 7, 10, 8]), redistribute(4));
    let mut crossed = Vec::new();
    while let GridMotion::Moving(transition) = &grid.motion {
        let (from, to) = (transition.from.clone(), grid.slots.clone());
        let left: Vec<Slot<u32>> = from
            .iter()
            .filter(|slot| to.contains(slot))
            .cloned()
            .collect();
        let landed: Vec<Slot<u32>> = to
            .iter()
            .filter(|slot| from.contains(slot))
            .cloned()
            .collect();
        if left != landed {
            crossed.push((from, to));
        }
        grid.advance();
    }
    assert!(crossed.is_empty(), "cells crossed: {crossed:?}");
    assert_eq!(
        shown(&grid),
        vec![
            TileContent::Group(9),
            TileContent::Group(7),
            TileContent::Group(10),
            TileContent::Group(8),
        ],
        "and the grid still arrives at the order it was asked for"
    );
}

#[test]
fn removing_refuses_to_close_a_cell_holding_a_command() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.settle_for_test();
    grid.remove();
    grid.settle_for_test();
    assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
}

#[test]
fn removing_closes_the_last_cell_that_plus_opened() {
    let mut grid = seeded_grid();
    grid.sync(&quiet(&[7]), redistribute(4));
    grid.add(redistribute(4));
    grid.remove();
    grid.settle_for_test();
    assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
}

#[test]
fn easing_pins_both_ends() {
    assert_eq!(eased(0), 0);
    assert_eq!(eased(PROGRESS_SCALE), PROGRESS_SCALE);
    assert!(eased(PROGRESS_SCALE / 2).abs_diff(PROGRESS_SCALE / 2) <= 1);
}

struct MotionPaintCells(TileDemands<u32>);

impl crate::TileCells<u32> for MotionPaintCells {
    fn summary_title(&self) -> &'static str { "summary" }
    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> { self.0.clone() }
    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}

/// Apply one key and inspect every point of every step it queues.
fn assert_key_keeps_unchanged_heights_one_way(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
    action: TileAction,
) {
    let contents = [TileContent::Summary, TileContent::Group(1)];
    let mut paths = vec![Vec::new(); contents.len()];
    grid.apply(action, growth);
    while matches!(grid.motion, GridMotion::Moving(_)) {
        for raw in 0..=PROGRESS_SCALE {
            let drawing = grid.drawing_at(area, growth, raw);
            for (content, path) in contents.iter().zip(&mut paths) {
                let mut pieces = drawing
                    .placements()
                    .filter(|placement| &placement.content == content);
                let piece = pieces.next().expect("the unchanged cell remains visible");
                assert!(
                    pieces.next().is_none(),
                    "the unchanged cell stays in its band"
                );
                path.push(piece.frame.clip().height);
            }
        }
        grid.advance();
    }
    for (content, heights) in contents.iter().zip(paths) {
        let (&start, &end) = heights
            .first()
            .zip(heights.last())
            .expect("the key queues a moving frame");
        let one_way = if start <= end {
            heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(one_way, "{content:?} reverses through {heights:?}");
    }
}

fn assert_piece_heights_move_one_way(before: &[u16], after: &[u16]) {
    let (mut grid, area, growth) = exact_height_motion(before, after);
    let mut heights = vec![Vec::new(); before.len()];
    for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
        let drawing = filled_height_drawing(&mut grid, area, growth, raw);
        for (piece, heights) in drawing.placements().zip(&mut heights) {
            heights.push(piece.frame.clip().height);
        }
    }
    for ((&start, &finish), heights) in before.iter().zip(after).zip(heights) {
        assert_eq!(heights.first().copied(), Some(start));
        assert_eq!(heights.last().copied(), Some(finish));
        let moves_one_way = if start <= finish {
            heights.windows(2).all(|pair| pair[0] <= pair[1])
        } else {
            heights.windows(2).all(|pair| pair[0] >= pair[1])
        };
        assert!(
            moves_one_way,
            "{start} to {finish} reverses through {heights:?}"
        );
    }
}

/// Settled cell rects in screen order, with collapsed motion pieces omitted.
fn endpoint_rects(drawing: &TileDrawing<u32>) -> Vec<Rect> {
    let mut rects = drawing
        .placements()
        .map(|placement| placement.frame.clip())
        .filter(|rect| rect.width > 1 && rect.height > 1)
        .collect::<Vec<_>>();
    rects.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
    rects
}

/// Every settled rect at one end of a moving grid.
fn settled_rects(grid: &Grid, count: usize) -> Vec<Rect> {
    let mut rects = (TABLE_CELL..=count)
        .filter_map(|cell| grid.cell(cell))
        .collect::<Vec<_>>();
    rects.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
    rects
}

/// Check both exact ends after exercising every standard motion snapshot.
fn assert_settled_motion_endpoints(label: &str, grid: &TileGrid<u32>, growth: TileGrowth) {
    assert!(
        matches!(grid.motion, GridMotion::Moving(_)),
        "{label} is moving"
    );
    let GridMotion::Moving(transition) = &grid.motion else {
        return;
    };
    let before = Grid::new(test_area(), &transition.held, growth, &grid.settings);
    let after_held = grid.drawn_held();
    let after = Grid::new(test_area(), &after_held, growth, &grid.settings);
    let drawings = motion_steps()
        .map(|raw| grid.drawing_at(test_area(), growth, raw))
        .collect::<Vec<_>>();
    assert_eq!(drawings.len(), 24);
    assert_eq!(
        endpoint_rects(&drawings[0]),
        settled_rects(&before, transition.held.rows.len()),
        "{label} starts on its before rects"
    );
    assert_eq!(
        endpoint_rects(&drawings[drawings.len() - 1]),
        settled_rects(&after, after_held.rows.len()),
        "{label} ends on its after rects"
    );
}

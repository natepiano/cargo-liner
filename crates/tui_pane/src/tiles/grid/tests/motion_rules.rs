use super::draw::TileMeasurements;
use super::*;
use crate::ColorMode;
use crate::TileCells;
use crate::tiles::constants::COLUMN_FOOT_TEST_AREA;
use crate::tiles::constants::COLUMN_FOOT_TEST_MIN_HEIGHT;
use crate::tiles::constants::COMMAND_OPENING_TEST_SPANS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_COMMAND_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_PROGRESS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_PROJECT_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_ROW;
use crate::tiles::constants::EMPTY_DEPARTURE_TEST_SPANS;
use crate::tiles::constants::LENDING_TEST_SUMMARY_TITLE;
use crate::tiles::constants::RESIDENT_EMPTY_TEST_SPANS;
use crate::tiles::constants::RESIDENT_EMPTY_TEST_START_SPANS;
use crate::tiles::constants::SLIDING_TEST_INITIAL_ROWS;
use crate::tiles::constants::SLIDING_TEST_INTERVALS;
use crate::tiles::constants::SUMMARY_STEP_TEST_ENTRY_ROWS;
use crate::tiles::constants::SUMMARY_STEP_TEST_HEADER_ROWS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MotionRuleKey {
    ResidentEmpty,
    CommandOpening,
    EmptyDeparture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixturePieceRole {
    Resident,
    EnteringClaimedCommand,
    EnteringQueuedCommand,
    EnteringEmpty,
    LeavingEmpty,
}

struct MotionRuleCells {
    entries:           u16,
    commands:          u32,
    departed_commands: u32,
}

impl TileCells<u32> for MotionRuleCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS),
            summary_width: 0,
            groups:        (self.departed_commands + 1..=self.commands)
                .map(|id| TileDemand {
                    id,
                    rows: usize::from(CONTENT_CHANGE_TEST_COMMAND_ROWS),
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary => {
                let whole = self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS;
                if area.height >= whole {
                    whole
                } else {
                    let header = area.height.min(SUMMARY_STEP_TEST_HEADER_ROWS);
                    header
                        + (area.height.saturating_sub(header) / SUMMARY_STEP_TEST_ENTRY_ROWS)
                            .min(self.entries)
                            * SUMMARY_STEP_TEST_ENTRY_ROWS
                }
            },
            TileContent::Group(id) if *id <= self.departed_commands => 0,
            TileContent::Group(_) => area.height.min(CONTENT_CHANGE_TEST_COMMAND_ROWS),
            TileContent::Empty(_) => 0,
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, area: Rect, _: Color) {
        for y in area.top()..area.top() + self.rows_drawn(content, area) {
            buffer[(area.x, y)].set_symbol(CONTENT_CHANGE_TEST_ROW);
        }
    }
}

fn motion_rule_frame(
    grid: &mut TileGrid<u32>,
    cells: &MotionRuleCells,
    raw: u32,
) -> (TileDrawing<u32>, Buffer) {
    let area = COLUMN_FOOT_TEST_AREA;
    let growth = redistribute(SLIDING_TEST_INITIAL_ROWS);
    let widths = grid.content_widths(area, growth);
    let mut demands = cells.demands(&widths);
    demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
    for group in &mut demands.groups {
        group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
    }
    if cells.departed_commands > 0 {
        grid.demands = demands.clone();
        grid.resolve_key_destination_depth();
    } else {
        grid.sync_rendered_demands(&demands, growth);
    }
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |content, area| cells.rows_drawn(content, area),
        |content, area| cells.rows_kept(content, area),
    );
    let drawing = grid.drawing_at(area, growth, raw);
    let mut buffer = Buffer::empty(area);
    draw::draw_placements(
        &mut buffer,
        area,
        &drawing,
        &TileMeasurements {
            demands: &demands,
            widths:  &widths,
        },
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    (drawing, buffer)
}

fn motion_rule_fixture(key: MotionRuleKey) -> (TileGrid<u32>, MotionRuleCells) {
    let growth = redistribute(SLIDING_TEST_INITIAL_ROWS);
    let (entries, commands, slots, spans) = match key {
        MotionRuleKey::ResidentEmpty => (
            5,
            5,
            vec![
                Slot::Group(1),
                Slot::Group(2),
                Slot::Group(3),
                Slot::Group(4),
                Slot::Group(5),
                Slot::Empty(1),
                Slot::Empty(2),
            ],
            RESIDENT_EMPTY_TEST_START_SPANS.as_slice(),
        ),
        MotionRuleKey::CommandOpening => (
            2,
            1,
            vec![Slot::Group(1)],
            COMMAND_OPENING_TEST_SPANS.as_slice(),
        ),
        MotionRuleKey::EmptyDeparture => (
            4,
            3,
            vec![
                Slot::Empty(0),
                Slot::Group(1),
                Slot::Group(2),
                Slot::Group(3),
            ],
            EMPTY_DEPARTURE_TEST_SPANS.as_slice(),
        ),
    };
    let mut cells = MotionRuleCells {
        entries,
        commands,
        departed_commands: u32::from(key == MotionRuleKey::ResidentEmpty),
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(COLUMN_FOOT_TEST_AREA.width);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    grid.set_layout(COLUMN_FOOT_TEST_AREA, growth);
    grid.slots = slots;
    grid.next_slot = 3;
    motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
    grid.settle_for_test();
    motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
    grid.record_settled_spans_for_test(spans);
    match key {
        MotionRuleKey::ResidentEmpty => {
            let mut target = grid.slots.clone();
            target.push(Slot::Empty(3));
            grid.queue_with_depth(
                vec![target],
                grid.depth,
                DestinationDepthStatus::AwaitingMeasuredWidths,
            );
            cells.entries = 4;
            cells.departed_commands = 1;
            let mut heights = RESIDENT_EMPTY_TEST_SPANS
                .iter()
                .map(|span| span + 1)
                .collect::<Vec<_>>();
            heights.push(1);
            grid.motion_heights = MotionHeightPath {
                area: COLUMN_FOOT_TEST_AREA,
                growth,
                progress: CONTENT_CHANGE_TEST_PROGRESS[0],
                piece_heights: heights,
            };
        },
        MotionRuleKey::CommandOpening => {
            cells.commands = 2;
            let mut demands = cells.demands(&[]);
            demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
            for group in &mut demands.groups {
                group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
            }
            grid.sync(&demands, growth);
        },
        MotionRuleKey::EmptyDeparture => {
            grid.focus = Focus::Cell(Slot::Empty(0));
            grid.apply(TileAction::Remove, growth);
        },
    }
    (grid, cells)
}

fn fixture_role(key: MotionRuleKey, piece: &TilePiece<u32>) -> FixturePieceRole {
    match piece.height_snap {
        PieceHeightSnap::Search { start: 1, .. } => match (key, &piece.placement.content) {
            (MotionRuleKey::CommandOpening, TileContent::Group(2)) => {
                FixturePieceRole::EnteringClaimedCommand
            },
            (MotionRuleKey::CommandOpening, TileContent::Group(3)) => {
                FixturePieceRole::EnteringQueuedCommand
            },
            _ => FixturePieceRole::EnteringEmpty,
        },
        PieceHeightSnap::Search { end: 1, .. } => FixturePieceRole::LeavingEmpty,
        _ => FixturePieceRole::Resident,
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MotionRuleAssertions {
    All,
    ResidentAllocation,
    EnteringSlide,
    LeavingSlide,
}

#[derive(Clone, Copy)]
struct MotionRuleFrame<'a> {
    key:      MotionRuleKey,
    checks:   MotionRuleAssertions,
    raw:      u32,
    cells:    &'a MotionRuleCells,
    drawing:  &'a TileDrawing<u32>,
    endpoint: &'a TileDrawing<u32>,
    buffer:   &'a Buffer,
}

impl MotionRuleFrame<'_> {
    fn assert_content(&self, tile: &TilePiece<u32>) {
        let Self {
            key,
            raw,
            cells,
            buffer,
            ..
        } = *self;
        let piece = &tile.placement;
        let role = fixture_role(key, tile);
        let clip = piece.frame.clip();
        let area = draw::content_area(frame_inner(clip));
        let rows = cells.rows_drawn(&piece.content, area);
        let whole = cells.rows_drawn(
            &piece.content,
            Rect {
                height: u16::MAX,
                ..area
            },
        );
        assert!(
            rows == area.height || rows == whole,
            "{key:?} {raw}: {role:?} {:?} has a blank row in {clip:?}",
            piece.content
        );
        if rows < whole && area.height > 0 {
            assert_eq!(
                buffer[(area.x, area.bottom() - 1)].symbol(),
                CONTENT_CHANGE_TEST_ROW
            );
        }
    }

    fn assert_resident_spare_rows(&self, tile: &TilePiece<u32>) {
        let Self {
            key,
            checks,
            raw,
            cells,
            drawing,
            endpoint,
            ..
        } = *self;
        let piece = &tile.placement;
        let role = fixture_role(key, tile);
        let clip = piece.frame.clip();
        let area = draw::content_area(frame_inner(clip));
        let rows = cells.rows_drawn(&piece.content, area);
        if matches!(
            checks,
            MotionRuleAssertions::All | MotionRuleAssertions::ResidentAllocation
        ) && role == FixturePieceRole::Resident
            && clip.height > COLUMN_FOOT_TEST_MIN_HEIGHT
            && rows < area.height
        {
            let end = endpoint
                .placements()
                .find(|end| end.content == piece.content)
                .map_or(0, |end| end.frame.clip().height);
            if clip.height > end
                || raw == PROGRESS_SCALE
                || (key == MotionRuleKey::ResidentEmpty && piece.content == TileContent::Group(1))
            {
                for mate in drawing
                    .pieces
                    .iter()
                    .filter(|mate| {
                        fixture_role(key, mate) == FixturePieceRole::Resident
                            && mate.placement.content != piece.content
                    })
                    .map(|mate| &mate.placement)
                {
                    let mate_area = draw::content_area(frame_inner(mate.frame.clip()));
                    let next = cells.rows_drawn(
                        &mate.content,
                        Rect {
                            height: mate_area.height + 1,
                            ..mate_area
                        },
                    );
                    assert!(
                        next <= cells.rows_drawn(&mate.content, mate_area),
                        "{key:?} {raw}: {:?} holds spare beside {:?}; {:?}",
                        piece.content,
                        mate.content,
                        drawing
                            .placements()
                            .map(|p| (&p.content, p.frame.clip()))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }

    fn record_heights(
        &self,
        tile: &TilePiece<u32>,
        paths: &mut Vec<(TileContent<u32>, FixturePieceRole, Vec<u16>)>,
    ) {
        let Self {
            key, checks, raw, ..
        } = *self;
        let piece = &tile.placement;
        let role = fixture_role(key, tile);
        let clip = piece.frame.clip();
        let height = clip.height;
        if let Some((_, _, heights)) = paths
            .iter_mut()
            .find(|(content, held_role, _)| *content == piece.content && *held_role == role)
        {
            let previous = heights.last().copied().unwrap_or(height);
            let step = if previous <= 1 || key == MotionRuleKey::ResidentEmpty {
                SUMMARY_STEP_TEST_ENTRY_ROWS
            } else {
                MOVING_HEIGHT_PROBE_STEP
            };
            match role {
                FixturePieceRole::EnteringClaimedCommand
                | FixturePieceRole::EnteringQueuedCommand
                | FixturePieceRole::EnteringEmpty
                    if matches!(
                        checks,
                        MotionRuleAssertions::All | MotionRuleAssertions::EnteringSlide
                    ) =>
                {
                    assert!(
                        height >= previous && height - previous <= step,
                        "{key:?} {raw}: {role:?} steps {previous} -> {height}"
                    );
                },
                FixturePieceRole::LeavingEmpty
                    if matches!(
                        checks,
                        MotionRuleAssertions::All | MotionRuleAssertions::LeavingSlide
                    ) =>
                {
                    assert!(
                        height <= previous,
                        "{key:?} {raw}: leaving piece grows {previous} -> {height}"
                    );
                },
                _ => {},
            }
            heights.push(height);
        } else {
            if matches!(
                role,
                FixturePieceRole::EnteringClaimedCommand
                    | FixturePieceRole::EnteringQueuedCommand
                    | FixturePieceRole::EnteringEmpty
            ) && matches!(
                checks,
                MotionRuleAssertions::All | MotionRuleAssertions::EnteringSlide
            ) {
                assert!(
                    height <= MIN_TILE_HEIGHT,
                    "{key:?} {raw}: {role:?} appears at {height}"
                );
            }
            paths.push((piece.content.clone(), role, vec![height]));
        }
        if matches!(
            checks,
            MotionRuleAssertions::All | MotionRuleAssertions::LeavingSlide
        ) && role == FixturePieceRole::LeavingEmpty
            && raw < PROGRESS_SCALE
        {
            assert!(
                draw::has_visible_body_row(piece.frame),
                "{key:?} {raw}: leaving cell vanishes before its clock ends"
            );
        }
    }
}

fn assert_motion_rules(key: MotionRuleKey, checks: MotionRuleAssertions) {
    let (mut grid, mut cells) = motion_rule_fixture(key);
    let clock = grid.motion_timing_for_test();
    let mut paths = Vec::<(TileContent<u32>, FixturePieceRole, Vec<u16>)>::new();
    let mut last = Vec::new();
    for sample in 0..=SLIDING_TEST_INTERVALS {
        let start = if key == MotionRuleKey::ResidentEmpty {
            CONTENT_CHANGE_TEST_PROGRESS[0]
        } else {
            0
        };
        let raw = start + sample * (PROGRESS_SCALE - start) / SLIDING_TEST_INTERVALS;
        match key {
            MotionRuleKey::ResidentEmpty if raw >= PROGRESS_SCALE / 2 => {
                cells.entries = 3;
                cells.departed_commands = 2;
            },
            MotionRuleKey::CommandOpening if raw >= PROGRESS_SCALE / 10 => {
                cells.entries = 3;
            },
            _ => {},
        }
        if key == MotionRuleKey::CommandOpening && sample == SLIDING_TEST_INTERVALS / 8 {
            cells.commands = 3;
            let widths = grid.content_widths(COLUMN_FOOT_TEST_AREA, grid.growth);
            let mut demands = cells.demands(&widths);
            demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
            for group in &mut demands.groups {
                group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
            }
            grid.sync(&demands, grid.growth);
            grid.apply(TileAction::Add, grid.growth);
        }
        let (drawing, buffer) = motion_rule_frame(&mut grid, &cells, raw);
        assert_eq!(clock, grid.motion_timing_for_test(), "{key:?} {raw}");
        let endpoint = grid.drawing_at(COLUMN_FOOT_TEST_AREA, grid.growth, PROGRESS_SCALE);
        let frame = MotionRuleFrame {
            key,
            checks,
            raw,
            cells: &cells,
            drawing: &drawing,
            endpoint: &endpoint,
            buffer: &buffer,
        };
        for tile in &drawing.pieces {
            frame.assert_content(tile);
            frame.assert_resident_spare_rows(tile);
            frame.record_heights(tile, &mut paths);
        }
        last = drawing
            .placements()
            .filter(|p| draw::has_visible_body_row(p.frame))
            .map(|p| (p.content.clone(), p.frame.clip()))
            .collect();
    }
    for (content, role, mut heights) in paths {
        if role != FixturePieceRole::Resident
            || !matches!(
                checks,
                MotionRuleAssertions::All | MotionRuleAssertions::ResidentAllocation
            )
        {
            continue;
        }
        heights.dedup();
        assert!(
            heights.windows(3).all(|h| (h[0] < h[1]) == (h[1] < h[2])),
            "{key:?}: resident {content:?} reverses {heights:?}"
        );
    }
    grid.finish_motion_for_test();
    let (settled, buffer) = motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
    let frame = MotionRuleFrame {
        key,
        checks,
        raw: PROGRESS_SCALE,
        cells: &cells,
        drawing: &settled,
        endpoint: &settled,
        buffer: &buffer,
    };
    for tile in &settled.pieces {
        frame.assert_content(tile);
        frame.assert_resident_spare_rows(tile);
    }
    assert_eq!(
        last,
        settled
            .placements()
            .filter(|p| draw::has_visible_body_row(p.frame))
            .map(|p| (p.content.clone(), p.frame.clip()))
            .collect::<Vec<_>>()
    );
    assert!(!grid.tick(), "{key:?} has a second motion");
}

#[test]
fn a_resident_empty_cell_takes_no_row_while_mates_cut() {
    assert_motion_rules(
        MotionRuleKey::ResidentEmpty,
        MotionRuleAssertions::ResidentAllocation,
    );
}
#[test]
fn a_command_cell_slides_in_when_its_content_arrives_mid_slide() {
    assert_motion_rules(
        MotionRuleKey::CommandOpening,
        MotionRuleAssertions::EnteringSlide,
    );
}
#[test]
fn an_empty_cell_slides_out_across_a_summary_depth_step() {
    assert_motion_rules(
        MotionRuleKey::EmptyDeparture,
        MotionRuleAssertions::LeavingSlide,
    );
}
#[test]
fn every_frame_of_a_key_keeps_the_motion_rules() {
    for key in [
        MotionRuleKey::ResidentEmpty,
        MotionRuleKey::CommandOpening,
        MotionRuleKey::EmptyDeparture,
    ] {
        assert_motion_rules(key, MotionRuleAssertions::All);
    }
}

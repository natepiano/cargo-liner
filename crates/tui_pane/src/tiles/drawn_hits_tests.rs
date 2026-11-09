//! Public cell lookup follows the last painted pieces.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::*;
use crate::ColorMode;
use crate::TileCells;

const AREA: Rect = Rect::new(0, 0, 80, 40);
const GROWTH: TileGrowth = TileGrowth {
    initial_rows:  4,
    fill:          TileFill::Redistribute,
    widen_summary: false,
};
const IDS: [u8; 3] = [1, 2, 3];
const HALF_PROGRESS: u32 = PROGRESS_SCALE / 2;

struct LetterCells;

impl TileCells<u8> for LetterCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u8>, u16)]) -> TileDemands<u8> { demands(&IDS) }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u8>, area: Rect, _: Color) {
        let letter = match content {
            TileContent::Summary => "S",
            TileContent::Group(1) => "A",
            TileContent::Group(2) => "B",
            TileContent::Group(_) => "C",
            TileContent::Empty(_) => "E",
        };
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buffer[(x, y)].set_symbol(letter);
            }
        }
    }
}

fn demands(ids: &[u8]) -> TileDemands<u8> {
    TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows: 0 }).collect(),
    }
}

fn opening_grid() -> TileGrid<u8> {
    let mut grid = TileGrid::new();
    grid.set_layout(AREA, GROWTH);
    grid.sync(&demands(&IDS), GROWTH);
    grid.settle_for_test();
    grid.apply(TileAction::Add, GROWTH);
    grid
}

fn paint(grid: &mut TileGrid<u8>, drawing: TileDrawing<u8>) -> Buffer {
    let mut buffer = Buffer::empty(AREA);
    let widths = grid.content_widths(AREA, GROWTH);
    let frames = draw::draw_placements(
        &mut buffer,
        AREA,
        &drawing,
        &TileMeasurements {
            demands: &grid.demands,
            widths:  &widths,
        },
        TileGridContents::Shown,
        &LetterCells,
        ColorMode::Colored,
    );
    grid.record_painted_body(drawing, TileGridContents::Shown, frames);
    buffer
}

fn letter_index(letter: &str) -> Option<usize> {
    match letter {
        "S" => Some(1),
        "A" => Some(2),
        "B" => Some(3),
        "C" => Some(4),
        "E" => Some(5),
        _ => None,
    }
}

#[test]
fn moving_piece_pixels_keep_their_cell_until_another_body_paint() {
    let mut grid = opening_grid();
    grid.hold_motion_at_half_for_test();
    let mut buffer = Buffer::empty(AREA);
    draw::draw_tile_grid(
        &mut buffer,
        &mut grid,
        AREA,
        GROWTH,
        TileGridContents::Shown,
        &LetterCells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    let mut checked = 0;
    for y in AREA.top()..AREA.bottom() {
        for x in AREA.left()..AREA.right() {
            if let Some(index) = letter_index(buffer[(x, y)].symbol()) {
                assert_eq!(
                    grid.cell_at(Position::new(x, y)),
                    Some(index),
                    "painted pixel at {x},{y}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "the moving frame paints content");
}

#[test]
fn a_destination_press_returns_the_piece_drawn_there() {
    let mut grid = opening_grid();
    let drawing = grid.drawing_at(AREA, GROWTH, HALF_PROGRESS);
    let buffer = paint(&mut grid, drawing);
    grid.settle_for_test();
    let destination = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    let mut displaced = 0;
    for piece in destination.pieces {
        let destination_index = match piece.identity {
            CellIdentity::Summary => TABLE_CELL,
            CellIdentity::Slot(ref slot) => cell_of(&grid.slots, slot).unwrap_or_default(),
        };
        let rect = piece.placement.frame.inner();
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                if let Some(painted_index) = letter_index(buffer[(x, y)].symbol())
                    && painted_index != destination_index
                {
                    assert_eq!(grid.cell_at(Position::new(x, y)), Some(painted_index));
                    displaced += 1;
                }
            }
        }
    }
    assert!(displaced > 0, "a destination overlaps another moving piece");
}

#[test]
fn clipped_away_piece_pixels_supply_no_cell() {
    let mut grid = opening_grid();
    grid.settle_for_test();
    let mut drawing = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    drawing
        .pieces
        .retain(|piece| piece.placement.content == TileContent::Group(1));
    assert_eq!(
        drawing.pieces.len(),
        1,
        "the clipped fixture retains one piece"
    );
    for piece in &mut drawing.pieces {
        let rect = piece.placement.frame.rect();
        let clip = Rect::new(
            rect.x,
            rect.y + rect.height / 2,
            rect.width,
            rect.height / 2,
        );
        piece.placement.frame = PaneFrame::shifted(rect, 0, clip);
    }
    let point = drawing
        .pieces
        .iter()
        .map(|piece| {
            let rect = piece.placement.frame.rect();
            Position::new(rect.x + 2, rect.y + 1)
        })
        .next()
        .unwrap_or_default();
    paint(&mut grid, drawing);
    assert_eq!(grid.cell_at(point), None);
}

#[test]
fn an_empty_body_paint_replaces_the_previous_hit_regions() {
    let mut grid = opening_grid();
    grid.settle_for_test();
    let drawing = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    let buffer = paint(&mut grid, drawing);
    let point = AREA
        .positions()
        .find(|&point| buffer[point].symbol() == "A")
        .unwrap_or_default();
    assert_eq!(grid.cell_at(point), Some(2));
    paint(
        &mut grid,
        TileDrawing {
            pieces:       Vec::new(),
            column_bands: Vec::new(),
        },
    );
    assert_eq!(grid.cell_at(point), None);
}

#[test]
fn public_lookup_uses_the_topmost_painted_piece() {
    let mut grid = opening_grid();
    grid.settle_for_test();
    let mut drawing = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    drawing
        .pieces
        .retain(|piece| matches!(piece.placement.content, TileContent::Group(1 | 2)));
    let frame = drawing.pieces[0].placement.frame;
    for piece in &mut drawing.pieces {
        piece.placement.frame = frame;
    }
    let point = Position::new(frame.inner().x + 1, frame.inner().y);
    let buffer = paint(&mut grid, drawing);
    assert_eq!(buffer[point].symbol(), "B");
    assert_eq!(grid.cell_at(point), Some(3));
}

#[test]
fn retired_painted_pixels_do_not_resolve_to_a_replacement_slot() {
    let mut grid = opening_grid();
    grid.settle_for_test();
    let drawing = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    let buffer = paint(&mut grid, drawing);
    let point = AREA
        .positions()
        .find(|&point| buffer[point].symbol() == "A")
        .unwrap_or_default();
    assert_eq!(grid.cell_at(point), Some(2));
    grid.sync(&demands(&[2, 3]), GROWTH);
    grid.settle_for_test();
    assert_eq!(grid.cell_at(point), None);
}

#[test]
fn content_identity_follows_the_current_slot_after_its_neighbor_leaves() {
    let mut grid = opening_grid();
    grid.settle_for_test();
    let drawing = grid.drawing_at(AREA, GROWTH, PROGRESS_SCALE);
    paint(&mut grid, drawing);
    grid.sync(&demands(&[2, 3]), GROWTH);
    grid.settle_for_test();
    let index = grid.cell_index(&TileContent::Group(3));
    assert_eq!(index, CellIndex::Current(3));
    assert_eq!(grid.cell_index(&TileContent::Group(1)), CellIndex::Missing);
    if let CellIndex::Current(index) = index {
        grid.focus_cell(index);
    }
    let focused = grid
        .placements(AREA, GROWTH)
        .into_iter()
        .find(|placement| placement.frame.is_focused())
        .map(|placement| placement.content);
    assert_eq!(focused, Some(TileContent::Group(3)));
}

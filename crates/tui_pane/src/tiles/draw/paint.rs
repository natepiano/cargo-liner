#[cfg(test)]
#[path = "paint_tests.rs"]
mod tests;

use ratatui::widgets::Widget;

use super::Buffer;
use super::CellReadout;
use super::CellReadoutInk;
use super::CellRowsReadout;
use super::Debug;
use super::GridLines;
use super::Line;
use super::PaneBorders;
use super::PaneChrome;
use super::PaneFrame;
use super::Paragraph;
use super::PieceName;
use super::Rect;
use super::Span;
use super::Style;
use super::SummaryFoot;
use super::SummaryFootDrawOutcome;
use super::TILE_FOOT_LEFT_INSET;
use super::TILE_NUMBER_INDENT;
use super::TileCells;
use super::TileContent;
use super::TileDrawing;
use super::TileGrid;
use super::TileGridContents;
use super::TileGrowth;
use super::TileMeasurements;
use super::content_area;
use super::default_pane_chrome;
use super::draw_clipped;
use super::draw_rows_readout;
use super::draw_rows_readout_after_foot;
use super::draw_summary_foot;
use super::frame_inner;
use super::prepare_grid;
use super::rows_readout_height;
use super::screen_ground;
use super::title_color;
/// Draw `grid` into `area`, its columns laid out by `growth`, with
/// `cells` saying what goes in each cell.
///
/// Records the layout, asks `cells` for each cell's rows at the width
/// the cell will be drawn at, adds the readout row to each, and settles
/// the grid against that before drawing it. The summary cell carries
/// [`TileCells::summary_title`]; every other cell carries one group, or
/// stands empty with its number.
///
/// Contents go down first and the frame over the top of them, because a
/// border belongs to the grid rather than to either cell it divides:
/// neighbours share one line, and [`GridLines`] is what knows the glyph
/// each crossing wants. The summary title goes on through
/// [`GridLines::add_titled`] for the same reason: a border a cell shares
/// is drawn by the pass that owns it, so anything written there has to
/// go in with it.
pub fn draw_tile_grid<Id: Clone + Eq + Debug>(
    buffer: &mut Buffer,
    grid: &mut TileGrid<Id>,
    area: Rect,
    growth: TileGrowth,
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
    mode: crate::ColorMode,
) {
    let prepared = prepare_grid(grid, area, growth, contents, cells);
    if prepared.drawing.pieces.is_empty() && prepared.drawing.column_bands.is_empty() {
        grid.record_painted_body(prepared.drawing, contents, Vec::new());
        return;
    }
    let frames = draw_placements(
        buffer,
        area,
        &prepared.drawing,
        &TileMeasurements {
            demands: &prepared.demands,
            widths:  &prepared.widths,
        },
        contents,
        cells,
        mode,
    );
    grid.record_painted_body(prepared.drawing, contents, frames);
}

/// Draw one grid snapshot: fixed column frames, then the cell pieces
/// moving inside them.
///
/// Every column band contributes its whole frame on every pass. A cell
/// piece contributes contents and lines only while at least one row of
/// its body is visible between its clip's frame rows, so an edge alone
/// cannot add a second line beside the column frame.
pub(in crate::tiles) fn draw_placements<Id: Clone + Eq + Debug>(
    buffer: &mut Buffer,
    area: Rect,
    drawing: &TileDrawing<Id>,
    measurements: &TileMeasurements<'_, Id>,
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
    mode: crate::ColorMode,
) -> Vec<Rect> {
    // Painted solid only while the work is shown: the attract screen
    // draws over bare panes, and keeps the terminal's own background.
    let painted_ground = if contents == TileGridContents::Shown {
        screen_ground()
    } else {
        None
    };
    if let Some(ground) = painted_ground {
        buffer.set_style(area, Style::default().bg(ground));
    }
    let mut grid_lines = GridLines::new(area);
    let mut summary_frames = Vec::new();
    let mut painted_summary_frames = Vec::new();
    for &column in &drawing.column_bands {
        grid_lines.add(PaneFrame::new(column));
    }
    for piece in &drawing.pieces {
        let placement = &piece.placement;
        if !has_visible_body_row(placement.frame) {
            continue;
        }
        let draws_name = piece.name == PieceName::Shown;
        // Contents retain the pane's own motion; its clip is the shared
        // topology the border lattice follows.
        let line_frame =
            PaneFrame::new(placement.frame.clip()).with_focus(placement.frame.is_focused());
        let cell_rows = measurements.rows(&placement.content);
        // The width the demand above was measured at, which is read
        // off the layout as it stood before `sync` -- the readout
        // carries it beside the cell's own so a cell measured at one
        // width and drawn at another says so.
        let demand_width = measurements.width(&placement.content);
        let content_rows = cell_rows.saturating_sub(usize::from(rows_readout_height(demand_width)));
        if contents == TileGridContents::Shown {
            let summary_foot = match &placement.content {
                TileContent::Summary => cells.summary_foot(),
                TileContent::Group(_) | TileContent::Empty(_) => SummaryFoot::Empty,
            };
            draw_clipped(buffer, placement.frame, |buffer, inner| {
                let ground =
                    PaneChrome::paint_tile_ground(buffer, inner, placement.frame.is_focused());
                draw_cell_with_rows_readout(
                    buffer,
                    inner,
                    || {
                        CellRowsReadout::new(
                            cells.rows_readout(
                                &placement.content,
                                content_area(inner),
                                content_rows,
                            ),
                            content_rows,
                        )
                    },
                    demand_width,
                    &summary_foot,
                    CellReadout::for_content(cells, &placement.content, ground),
                    |buffer, inner| match &placement.content {
                        TileContent::Empty(number) if draws_name => {
                            draw_number(buffer, *number, inner);
                        },
                        TileContent::Empty(_) => {},
                        TileContent::Summary | TileContent::Group(_) => {
                            cells.draw(buffer, &placement.content, inner, ground);
                        },
                    },
                );
            });
        }
        match &placement.content {
            TileContent::Summary => {
                summary_frames.push(line_frame.rect());
                if draws_name {
                    grid_lines.add_titled(line_frame, cells.summary_title());
                } else {
                    grid_lines.add(line_frame);
                }
                if contents == TileGridContents::Shown {
                    painted_summary_frames.push(line_frame.rect());
                    for label in cells.summary_labels(line_frame.rect()) {
                        grid_lines.add_label(line_frame, label);
                    }
                }
            },
            // A group's title names what the cell holds, so it goes with
            // the contents.
            TileContent::Group(id) => match cells
                .group_title(id)
                .filter(|_| draws_name && contents == TileGridContents::Shown)
            {
                Some(title) => grid_lines.add_titled(line_frame, title),
                None => grid_lines.add(line_frame),
            },
            TileContent::Empty(_) => grid_lines.add(line_frame),
        }
    }
    for rect in summary_frames {
        grid_lines.open(rect);
    }
    // Neighbouring tiles meet on one line, so no cell belongs to a
    // single tile. Focus is the background tint under a tile's contents,
    // and lights the tile's border only where the screen is transparent
    // and no tint is painted.
    let chrome = default_pane_chrome(mode);
    let chrome = painted_ground.map_or(chrome, |ground| PaneChrome {
        inactive_border: chrome.inactive_border.bg(ground),
        ..chrome
    });
    grid_lines.render(buffer, chrome, PaneBorders::Shared);
    painted_summary_frames
}

/// Whether a placement has a pane-interior row visible between its
/// clip's frame rows.
pub(in crate::tiles) fn has_visible_body_row(frame: PaneFrame) -> bool {
    let body = frame.inner();
    let clip_body = frame_inner(frame.clip());
    if body.is_empty() || clip_body.is_empty() {
        return false;
    }
    let shares_columns = body.left() < clip_body.right() && clip_body.left() < body.right();
    let shifted_top = i64::from(body.top()) + i64::from(frame.shift());
    let shifted_bottom = i64::from(body.bottom()) + i64::from(frame.shift());
    shares_columns
        && shifted_top < i64::from(clip_body.bottom())
        && shifted_bottom > i64::from(clip_body.top())
}

/// Whether a moving piece can show the row on which an empty cell is named.
pub(in crate::tiles) fn name_row_is_visible(frame: PaneFrame) -> bool {
    let body = frame.inner();
    let clip_body = frame_inner(frame.clip());
    if body.is_empty() || clip_body.is_empty() {
        return false;
    }
    let row = i64::from(body.top()) + i64::from(frame.shift());
    body.left() < clip_body.right()
        && clip_body.left() < body.right()
        && row >= i64::from(clip_body.top())
        && row < i64::from(clip_body.bottom())
}

/// Draw one cell's interior: its contents, then the readout along its
/// foot.
///
/// The readout's row is reserved before the contents go in, so `draw`
/// is handed the part of `inner` above it, and is not called at all
/// when that part is empty. [`TileContent::Empty`] never reaches `draw`:
/// its number is drawn here instead.
///
/// `rows` is what the contents ask for, the readout row not included,
/// and `measured_at` is the width they were measured at. The readout
/// writes `rows` green while the contents fit above it and red once
/// they do not, then the cell's own rows and columns, with
/// `measured_at` between them whenever it is not the cell's width.
pub fn draw_tile_cell<Id>(
    buffer: &mut Buffer,
    content: &TileContent<Id>,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    draw_cell(
        buffer,
        inner,
        rows,
        measured_at,
        &SummaryFoot::Empty,
        CellReadoutInk::Unfaded,
        |buffer, inner| match content {
            TileContent::Empty(number) => draw_number(buffer, *number, inner),
            TileContent::Summary | TileContent::Group(_) => draw(buffer, inner),
        },
    );
}

/// Draw one cell with the app's summary foot sharing its readout row.
pub(super) fn draw_cell(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    summary_foot: &SummaryFoot,
    readout: impl Into<CellReadout>,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    draw_cell_with_rows_readout(
        buffer,
        inner,
        || CellRowsReadout::count(rows),
        measured_at,
        summary_foot,
        readout,
        draw,
    );
}

/// Read the selected row after painting has resolved this allocation's surface.
fn draw_cell_with_rows_readout(
    buffer: &mut Buffer,
    inner: Rect,
    rows: impl FnOnce() -> CellRowsReadout,
    measured_at: u16,
    summary_foot: &SummaryFoot,
    readout: impl Into<CellReadout>,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    let CellReadout { extent, ink } = readout.into();
    let contents = content_area(inner);
    if !contents.is_empty() {
        draw(buffer, contents);
    }
    let rows = rows();
    let foot_draw_outcome = match summary_foot {
        SummaryFoot::Empty => SummaryFootDrawOutcome::NotDrawn,
        SummaryFoot::Text(line) => draw_summary_foot(buffer, inner, line, ink),
        SummaryFoot::WithOptionalSuffix { prefix, suffix } => {
            let mut line = prefix.clone();
            if prefix.width().saturating_add(suffix.width())
                <= usize::from(inner.width.saturating_sub(TILE_FOOT_LEFT_INSET))
            {
                line.spans.extend(suffix.spans.iter().cloned());
            }
            draw_summary_foot(buffer, inner, &line, ink)
        },
    };
    match foot_draw_outcome {
        SummaryFootDrawOutcome::NotDrawn => {
            draw_rows_readout(buffer, inner, rows, measured_at, extent, ink);
        },
        SummaryFootDrawOutcome::DrawnWhole { width } => {
            draw_rows_readout_after_foot(buffer, inner, rows, measured_at, width, extent, ink);
        },
    }
}

/// A cell opened with `+` that no command has claimed: its number, on
/// the first row it has to give.
fn draw_number(buffer: &mut Buffer, number: usize, inner: Rect) {
    Paragraph::new(Line::from(vec![
        Span::raw(TILE_NUMBER_INDENT),
        Span::styled(number.to_string(), Style::default().fg(title_color())),
    ]))
    .render(Rect { height: 1, ..inner }, buffer);
}

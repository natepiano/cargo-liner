//! Drawing the tile grid: each cell's contents, the readout along its
//! foot, and the borders over the top of them.
//!
//! [`TileCells`] is the app's part -- what each cell asks for and what
//! goes inside it. Everything else is here: the readout row every cell
//! reserves, the number an empty cell carries, and the one pass of
//! [`GridLines`] that draws every border the cells share.

use std::fmt::Debug;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::constants::TILE_NUMBER_INDENT;
use super::constants::TILE_ROWS_CELL_LABEL;
use super::constants::TILE_ROWS_CELL_SEPARATOR;
use super::constants::TILE_ROWS_CONTENT_LABEL;
use super::constants::TILE_ROWS_READOUT_HEIGHT;
use super::constants::TILE_ROWS_RIGHT_INSET;
use super::constants::TILE_ROWS_WIDTH_LABEL;
use super::grid::TileContent;
use super::grid::TileDemands;
use super::grid::TileGrid;
use crate::GridLines;
use crate::PaneBorders;
use crate::PaneFrameLabel;
use crate::default_pane_chrome;
use crate::draw_clipped;
use crate::error_color;
use crate::label_color;
use crate::pane_background;
use crate::success_color;
use crate::text_default;
use crate::title_color;

/// Whether the cells are drawn with what is in them.
///
/// Hidden leaves the frames, the borders and the summary title and
/// takes away the contents, the readouts and the summary's border
/// labels -- everything that is a number rather than a place. It is
/// what the grid looks like while something else is arriving over it or
/// leaving it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileGridContents {
    /// Draw what each cell holds.
    Shown,
    /// Draw the cells and nothing in them.
    Hidden,
}

/// What an app draws inside the cells of a [`TileGrid`].
///
/// [`draw_tile_grid`] owns the arrangement, the borders, the readout
/// along the foot of every cell and the number an empty cell carries.
/// This is the rest: how many rows each cell's contents ask for, and
/// the contents themselves.
pub trait TileCells<Id> {
    /// The title written into the summary cell's top border.
    fn summary_title(&self) -> &str;

    /// Rows each cell's contents would take given all the room they
    /// want, each measured at the width `widths` gives that cell.
    ///
    /// Content rows only: the grid adds the readout row itself. A group
    /// `widths` does not carry yet -- one first seen on this frame --
    /// belongs at the narrowest width in `widths`, which is the width it
    /// will have once the grid has opened a cell for it.
    fn demands(&self, widths: &[(TileContent<Id>, u16)]) -> TileDemands<Id>;

    /// Draw `content` into `inner`, the part of its cell above the
    /// readout. `ground` is the colour the cell is painted on.
    ///
    /// Never called for [`TileContent::Empty`], whose number the grid
    /// draws itself.
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<Id>, inner: Rect, ground: Color);

    /// Labels written along the summary cell's top border after its
    /// title, given the summary cell's whole box. None unless the app
    /// has some.
    fn summary_labels(&self, _rect: Rect) -> Vec<PaneFrameLabel> { Vec::new() }
}

/// Draw `grid` into `area`, with `cells` saying what goes in each cell.
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
    initial_rows: usize,
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
) {
    grid.set_layout(area, initial_rows);
    let widths = grid.content_widths(area, initial_rows);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, initial_rows);
    let placements = grid.placements(area, initial_rows);
    let mut grid_lines = GridLines::new(area);
    for placement in &placements {
        // The ground a fading row is carried toward is the one its own
        // cell is painted on, which focus moves.
        let ground = pane_background(placement.frame.is_focused());
        let cell_rows = match &placement.content {
            TileContent::Summary => demands.summary,
            TileContent::Group(id) => demands.rows_for(id),
            TileContent::Empty(_) => 0,
        };
        // The width the demand above was measured at, which is read off
        // the layout as it stood before `sync` -- the readout carries it
        // beside the cell's own so a cell measured at one width and
        // drawn at another says so rather than only looking wrong.
        let demand_width = measured_width(&widths, &placement.content);
        let content_rows = cell_rows.saturating_sub(usize::from(rows_readout_height(demand_width)));
        if contents == TileGridContents::Shown {
            draw_clipped(buffer, placement.frame, |buffer, inner| {
                draw_tile_cell(
                    buffer,
                    &placement.content,
                    inner,
                    content_rows,
                    demand_width,
                    |buffer, inner| cells.draw(buffer, &placement.content, inner, ground),
                );
            });
        }
        match &placement.content {
            TileContent::Summary => {
                grid_lines.add_titled(placement.frame, cells.summary_title());
                if contents == TileGridContents::Shown {
                    for label in cells.summary_labels(placement.frame.rect()) {
                        grid_lines.add_label(placement.frame, label);
                    }
                }
            },
            TileContent::Group(_) | TileContent::Empty(_) => {
                grid_lines.add(placement.frame);
            },
        }
    }
    // Neighbouring tiles meet on one line, so no cell belongs to a
    // single tile and none of them can carry focus. Focus is the
    // background tint under a tile's contents instead.
    grid_lines.render(buffer, default_pane_chrome(), PaneBorders::Shared);
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
    let contents = content_area(inner);
    if !contents.is_empty() {
        match content {
            TileContent::Empty(number) => draw_number(buffer, *number, contents),
            TileContent::Summary | TileContent::Group(_) => draw(buffer, contents),
        }
    }
    draw_rows_readout(buffer, inner, rows, measured_at);
}

/// Add the readout's row to what every cell asks for.
///
/// Each cell gets it at the width its contents were measured at, and
/// only where that width can display the readout -- the same width
/// [`draw_tile_grid`] goes on to write in the readout.
fn add_readout_rows<Id: Clone + Eq>(
    demands: &mut TileDemands<Id>,
    widths: &[(TileContent<Id>, u16)],
) {
    let summary_width = measured_width(widths, &TileContent::Summary);
    demands.summary = demands
        .summary
        .saturating_add(usize::from(rows_readout_height(summary_width)));
    for demand in &mut demands.groups {
        let width = measured_width(widths, &TileContent::Group(demand.id.clone()));
        demand.rows = demand
            .rows
            .saturating_add(usize::from(rows_readout_height(width)));
    }
}

/// The width `content` was measured at: its own entry in `widths`, or
/// the narrowest cell on screen for a group that has no cell yet, which
/// is the width it will have once the grid has opened one for it.
fn measured_width<Id: Eq>(widths: &[(TileContent<Id>, u16)], content: &TileContent<Id>) -> u16 {
    widths
        .iter()
        .find(|(measured, _)| measured == content)
        .map_or_else(
            || {
                widths
                    .iter()
                    .map(|&(_, width)| width)
                    .min()
                    .unwrap_or_default()
            },
            |&(_, width)| width,
        )
}

/// Write what a cell's contents ask for against what the cell was
/// given, along the separate readout row at the foot of the cell.
///
/// `rows` is the unrounded count the contents would take if nothing
/// stopped them. [`add_readout_rows`] adds the readout row before
/// [`TileGrid`] rounds to a step of demand and divides a column by it.
/// `inner.height` is the rows the cell actually has to draw into, one
/// short of its allotment wherever it shares a border with the cell
/// below. The count is written green while the contents fit above the
/// readout and red once they do not.
///
/// The cell's own rows and columns follow, and the width the demand was
/// settled at is written between them only when the two widths differ.
/// The demand is measured against the layout as it stood before
/// [`TileGrid::sync`] and the cell is drawn at whatever the layout
/// became after it -- and a line of text wraps, which makes a demand
/// taken at half the width very nearly twice the rows. So the two
/// widths agreeing is worth no room at all, while the two disagreeing
/// is the whole of what separates a cell asking for too much from a
/// cell asking against the wrong ruler, and is written red where it
/// appears.
fn draw_rows_readout(buffer: &mut Buffer, inner: Rect, rows: usize, measured_at: u16) {
    let reading = if rows <= usize::from(content_area(inner).height) {
        success_color()
    } else {
        error_color()
    };
    let mut spans = vec![
        Span::styled(TILE_ROWS_CONTENT_LABEL, Style::default().fg(label_color())),
        Span::styled(rows.to_string(), Style::default().fg(reading)),
    ];
    if measured_at != inner.width {
        spans.push(Span::styled(
            TILE_ROWS_WIDTH_LABEL,
            Style::default().fg(label_color()),
        ));
        spans.push(Span::styled(
            measured_at.to_string(),
            Style::default().fg(error_color()),
        ));
    }
    spans.extend([
        Span::styled(TILE_ROWS_CELL_LABEL, Style::default().fg(label_color())),
        Span::styled(
            inner.height.to_string(),
            Style::default().fg(text_default()),
        ),
        Span::styled(TILE_ROWS_CELL_SEPARATOR, Style::default().fg(label_color())),
        Span::styled(inner.width.to_string(), Style::default().fg(text_default())),
    ]);
    let line = Line::from(spans);
    let Some(area) = readout_area(inner, u16::try_from(line.width()).unwrap_or(u16::MAX)) else {
        return;
    };
    Paragraph::new(line).render(area, buffer);
}

/// The cell interior above the readout, or the whole interior when it cannot show one.
fn content_area(inner: Rect) -> Rect {
    readout_area(inner, inner.width).map_or(inner, |readout| Rect {
        height: readout.y.saturating_sub(inner.y),
        ..inner
    })
}

/// Rows reserved for the readout at a width that can display it.
const fn rows_readout_height(width: u16) -> u16 {
    if width > TILE_ROWS_RIGHT_INSET {
        TILE_ROWS_READOUT_HEIGHT
    } else {
        0
    }
}

/// The last row of a cell's interior, right-aligned and held off the
/// border, or `None` when the cell has no room for the readout at all.
fn readout_area(inner: Rect, width: u16) -> Option<Rect> {
    let room = inner.width.saturating_sub(TILE_ROWS_RIGHT_INSET);
    let height = rows_readout_height(inner.width);
    if height == 0 || inner.height < height {
        return None;
    }
    let width = width.min(room);
    Some(Rect {
        x: inner
            .right()
            .saturating_sub(TILE_ROWS_RIGHT_INSET)
            .saturating_sub(width),
        y: inner.bottom().saturating_sub(height),
        width,
        height,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One row of `buffer` as text, with the blanks to the right of it
    /// trimmed off.
    fn buffer_line(buffer: &Buffer, y: u16) -> String {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        line.trim_end().to_string()
    }

    #[test]
    fn an_empty_tile_number_needs_a_row_above_the_readout() {
        let number = 42;
        for height in [TILE_ROWS_READOUT_HEIGHT, TILE_ROWS_READOUT_HEIGHT + 1] {
            let inner = Rect::new(0, 0, 80, height);
            let mut buffer = Buffer::empty(inner);
            draw_tile_cell(
                &mut buffer,
                &TileContent::<u32>::Empty(number),
                inner,
                0,
                inner.width,
                |_, _| {},
            );

            assert!(
                buffer_line(&buffer, height - 1).contains(TILE_ROWS_CONTENT_LABEL),
                "the readout occupies the last interior row"
            );
            assert_eq!(
                buffer_line(&buffer, 0).contains(&number.to_string()),
                height > TILE_ROWS_READOUT_HEIGHT,
                "the tile number draws only when the readout leaves a content row"
            );
        }
    }
}

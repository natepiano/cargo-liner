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

use super::constants::TILE_FOOT_GAP;
use super::constants::TILE_FOOT_LEFT_INSET;
use super::constants::TILE_NUMBER_INDENT;
use super::constants::TILE_ROWS_CELL_LABEL;
use super::constants::TILE_ROWS_CELL_SEPARATOR;
use super::constants::TILE_ROWS_CONTENT_LABEL;
use super::constants::TILE_ROWS_READOUT_HEIGHT;
use super::constants::TILE_ROWS_RIGHT_INSET;
use super::constants::TILE_ROWS_WIDTH_LABEL;
use super::grid::GridDisplay;
use super::grid::PieceName;
use super::grid::TileContent;
use super::grid::TileDemands;
use super::grid::TileDrawing;
use super::grid::TileGrid;
use super::growth::TileGrowth;
use crate::GridLines;
use crate::PaneBorders;
use crate::PaneChrome;
use crate::PaneFrame;
use crate::PaneFrameLabel;
use crate::default_pane_chrome;
use crate::draw_clipped;
use crate::error_color;
use crate::frame_inner;
use crate::label_color;
use crate::pane_background;
use crate::screen_ground;
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

/// What the app writes at the left end of the summary cell's readout row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SummaryFoot {
    /// The app writes nothing there.
    Empty,
    /// The app's text, drawn whole from the left inset or not at all.
    Text(Line<'static>),
}

/// Whether the summary foot was drawn without clipping.
enum SummaryFootDrawOutcome {
    NotDrawn,
    DrawnWhole { width: u16 },
}

/// Whether a cell has a distinct row for its foot readout.
enum ReadoutRow {
    /// The last interior row, held inside the right inset.
    Reserved(Rect),
    /// The cell has one interior row, or is too narrow for a readout, so
    /// every row belongs to its contents.
    CellTooSmall,
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

    /// Text drawn whole or not at all at the left end of the summary
    /// cell's readout row.
    fn summary_foot(&self) -> SummaryFoot { SummaryFoot::Empty }

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

    /// The title written into the top border of the cell drawing `id`,
    /// set against the corner the way [`summary_title`] is, and left off
    /// while the contents are hidden. None unless the app names its
    /// groups there.
    ///
    /// The span's style is patched over the chrome's title style, so a
    /// title that sets only a colour keeps the weight focus gives it,
    /// and an unstyled one is drawn as the summary's title is.
    ///
    /// [`summary_title`]: Self::summary_title
    fn group_title(&self, _id: &Id) -> Option<Span<'static>> { None }
}

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
) {
    grid.set_layout(area, growth);
    let measured_display = grid.display();
    let mut widths = measurement_widths(grid, area, growth, measured_display);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, growth);
    let display = grid.display();
    if display != measured_display {
        widths = measurement_widths(grid, area, growth, display);
        demands = cells.demands(&widths);
        add_readout_rows(&mut demands, &widths);
    }
    if display == GridDisplay::SummaryAlone
        && (contents == TileGridContents::Hidden || frame_inner(area).is_empty())
    {
        return;
    }
    let drawing = grid.drawing(area, growth);
    draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells);
}

/// Widths used to measure the display that will be drawn this frame.
fn measurement_widths<Id: Clone + Eq + Debug>(
    grid: &TileGrid<Id>,
    area: Rect,
    growth: TileGrowth,
    display: GridDisplay,
) -> Vec<(TileContent<Id>, u16)> {
    match display {
        GridDisplay::Cells => grid.content_widths(area, growth),
        GridDisplay::SummaryAlone => {
            vec![(TileContent::Summary, frame_inner(area).width)]
        },
    }
}

/// Draw one grid snapshot: fixed column frames, then the cell pieces
/// moving inside them.
///
/// Every column band contributes its whole frame on every pass. A cell
/// piece contributes contents and lines only while at least one row of
/// its body is visible between its clip's frame rows, so an edge alone
/// cannot add a second line beside the column frame.
fn draw_placements<Id: Clone + Eq + Debug>(
    buffer: &mut Buffer,
    area: Rect,
    drawing: &TileDrawing<Id>,
    demands: &TileDemands<Id>,
    widths: &[(TileContent<Id>, u16)],
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
) {
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
    for &column in &drawing.column_bands {
        grid_lines.add(PaneFrame::new(column));
    }
    for piece in &drawing.pieces {
        let placement = &piece.placement;
        let visible_body = has_visible_body_row(placement.frame);
        let draws_name = piece.name == PieceName::Shown;
        if !visible_body && !draws_name {
            continue;
        }
        // Contents retain the pane's own motion; its clip is the shared
        // topology the border lattice follows.
        let line_frame =
            PaneFrame::new(placement.frame.clip()).with_focus(placement.frame.is_focused());
        if visible_body {
            // The ground a fading row is carried toward is the one its
            // own cell is painted on, which focus moves.
            let ground = pane_background(placement.frame.is_focused());
            let cell_rows = match &placement.content {
                TileContent::Summary => demands.summary,
                TileContent::Group(id) => demands.rows_for(id),
                TileContent::Empty(_) => 0,
            };
            // The width the demand above was measured at, which is read
            // off the layout as it stood before `sync` -- the readout
            // carries it beside the cell's own so a cell measured at one
            // width and drawn at another says so.
            let demand_width = measured_width(widths, &placement.content);
            let content_rows =
                cell_rows.saturating_sub(usize::from(rows_readout_height(demand_width)));
            if contents == TileGridContents::Shown {
                let summary_foot = match &placement.content {
                    TileContent::Summary => cells.summary_foot(),
                    TileContent::Group(_) | TileContent::Empty(_) => SummaryFoot::Empty,
                };
                draw_clipped(buffer, placement.frame, |buffer, inner| {
                    draw_cell(
                        buffer,
                        inner,
                        content_rows,
                        demand_width,
                        &summary_foot,
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
        }
        match &placement.content {
            TileContent::Summary => {
                if draws_name {
                    grid_lines.add_titled(line_frame, cells.summary_title());
                } else if visible_body {
                    grid_lines.add(line_frame);
                }
                if visible_body && contents == TileGridContents::Shown {
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
                None if visible_body => grid_lines.add(line_frame),
                None => {},
            },
            TileContent::Empty(_) if visible_body => {
                grid_lines.add(line_frame);
            },
            TileContent::Empty(_) => {},
        }
    }
    // Neighbouring tiles meet on one line, so no cell belongs to a
    // single tile. Focus is the background tint under a tile's contents,
    // and lights the tile's border only where the screen is transparent
    // and no tint is painted.
    let chrome = default_pane_chrome();
    let chrome = painted_ground.map_or(chrome, |ground| PaneChrome {
        inactive_border: chrome.inactive_border.bg(ground),
        ..chrome
    });
    grid_lines.render(buffer, chrome, PaneBorders::Shared);
}

/// Whether a placement has a pane-interior row visible between its
/// clip's frame rows.
fn has_visible_body_row(frame: PaneFrame) -> bool {
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
pub(super) fn name_row_is_visible(frame: PaneFrame) -> bool {
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
        |buffer, inner| match content {
            TileContent::Empty(number) => draw_number(buffer, *number, inner),
            TileContent::Summary | TileContent::Group(_) => draw(buffer, inner),
        },
    );
}

/// Draw one cell with the app's summary foot sharing its readout row.
fn draw_cell(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    summary_foot: &SummaryFoot,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    let contents = content_area(inner);
    if !contents.is_empty() {
        draw(buffer, contents);
    }
    let foot_draw_outcome = match summary_foot {
        SummaryFoot::Empty => SummaryFootDrawOutcome::NotDrawn,
        SummaryFoot::Text(line) => draw_summary_foot(buffer, inner, line),
    };
    match foot_draw_outcome {
        SummaryFootDrawOutcome::NotDrawn => {
            draw_rows_readout(buffer, inner, rows, measured_at);
        },
        SummaryFootDrawOutcome::DrawnWhole { width } => {
            draw_rows_readout_after_foot(buffer, inner, rows, measured_at, width);
        },
    }
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
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return;
    };
    if let Some(line) = rows_readout_lines(inner, rows, measured_at)
        .into_iter()
        .find(|line| line.width() <= usize::from(readout.width))
    {
        draw_rows_readout_line(buffer, readout, line);
    }
}

/// Draw the rows readout only when its whole line fits after the summary foot.
fn draw_rows_readout_after_foot(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    foot_width: u16,
) {
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return;
    };
    let room = readout
        .width
        .saturating_sub(TILE_FOOT_LEFT_INSET)
        .saturating_sub(foot_width)
        .saturating_sub(TILE_FOOT_GAP);
    if let Some(line) = rows_readout_lines(inner, rows, measured_at)
        .into_iter()
        .find(|line| line.width() <= usize::from(room))
    {
        draw_rows_readout_line(buffer, readout, line);
    }
}

/// Build the styled readouts that fit progressively narrower cells.
fn rows_readout_lines(inner: Rect, rows: usize, measured_at: u16) -> Vec<Line<'static>> {
    let reading = if rows <= usize::from(content_area(inner).height) {
        success_color()
    } else {
        error_color()
    };
    let content = vec![
        Span::styled(TILE_ROWS_CONTENT_LABEL, Style::default().fg(label_color())),
        Span::styled(rows.to_string(), Style::default().fg(reading)),
    ];
    let mut whole = content.clone();
    if measured_at != inner.width {
        whole.push(Span::styled(
            TILE_ROWS_WIDTH_LABEL,
            Style::default().fg(label_color()),
        ));
        whole.push(Span::styled(
            measured_at.to_string(),
            Style::default().fg(error_color()),
        ));
    }
    whole.extend([
        Span::styled(TILE_ROWS_CELL_LABEL, Style::default().fg(label_color())),
        Span::styled(
            inner.height.to_string(),
            Style::default().fg(text_default()),
        ),
        Span::styled(TILE_ROWS_CELL_SEPARATOR, Style::default().fg(label_color())),
        Span::styled(inner.width.to_string(), Style::default().fg(text_default())),
    ]);
    let mut lines = vec![Line::from(whole)];
    if measured_at != inner.width {
        let mut measured = content.clone();
        measured.extend([
            Span::styled(TILE_ROWS_WIDTH_LABEL, Style::default().fg(label_color())),
            Span::styled(measured_at.to_string(), Style::default().fg(error_color())),
        ]);
        lines.push(Line::from(measured));
    }
    lines.push(Line::from(content));
    lines
}

/// Right-align one rows readout that already fits its reserved row.
fn draw_rows_readout_line(buffer: &mut Buffer, readout: Rect, line: Line<'static>) {
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    if width > readout.width {
        return;
    }
    Paragraph::new(line).render(
        Rect {
            x: readout.right().saturating_sub(width),
            width,
            ..readout
        },
        buffer,
    );
}

/// Draw the app's whole text at the left end of the readout row, or nothing.
fn draw_summary_foot(
    buffer: &mut Buffer,
    inner: Rect,
    line: &Line<'static>,
) -> SummaryFootDrawOutcome {
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return SummaryFootDrawOutcome::NotDrawn;
    };
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    if width > inner.width.saturating_sub(TILE_FOOT_LEFT_INSET) {
        return SummaryFootDrawOutcome::NotDrawn;
    }
    Paragraph::new(line.clone()).render(
        Rect {
            x: inner.x.saturating_add(TILE_FOOT_LEFT_INSET),
            y: readout.y,
            width,
            height: readout.height,
        },
        buffer,
    );
    SummaryFootDrawOutcome::DrawnWhole { width }
}

/// The cell interior above the readout, or the whole interior when it cannot show one.
const fn content_area(inner: Rect) -> Rect {
    match readout_row(inner) {
        ReadoutRow::Reserved(readout) => Rect {
            height: readout.y.saturating_sub(inner.y),
            ..inner
        },
        ReadoutRow::CellTooSmall => inner,
    }
}

/// Rows reserved for the readout at a width that can display it.
const fn rows_readout_height(width: u16) -> u16 {
    if width > TILE_ROWS_RIGHT_INSET {
        TILE_ROWS_READOUT_HEIGHT
    } else {
        0
    }
}

/// The last interior row inside the right inset, when a content row remains.
const fn readout_row(inner: Rect) -> ReadoutRow {
    let width = inner.width.saturating_sub(TILE_ROWS_RIGHT_INSET);
    let height = rows_readout_height(inner.width);
    if height == 0 || inner.height <= height {
        return ReadoutRow::CellTooSmall;
    }
    ReadoutRow::Reserved(Rect {
        x: inner.x,
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
    use std::ops::Range;

    use ratatui::symbols::line;

    use super::*;
    use crate::TileDemand;
    use crate::TileFill;
    use crate::tiles::constants::MIN_TILE_HEIGHT;
    use crate::tiles::constants::PROGRESS_SCALE;
    use crate::tiles::grid::TilePiece;
    use crate::tiles::grid::TilePlacement;

    const FOOT_TEXT: &str = "mem 2.5G";
    const GROUP_TITLE_GLYPHS: &[u8] =
        b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    const MOTION_SNAPSHOTS: u32 = 24;
    const TITLE_AREA: Rect = Rect::new(0, 0, 200, 50);
    const TEST_TILE_WIDTH: u16 = 40;
    const CLOSING_GROWTH: TileGrowth = TileGrowth {
        initial_rows:  3,
        fill:          TileFill::AddNew,
        widen_summary: false,
    };
    const STAYING_GROWTH: TileGrowth = TileGrowth {
        initial_rows:  4,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };

    struct StubCells {
        summary_foot: SummaryFoot,
        groups:       Vec<u32>,
    }

    impl TileCells<u32> for StubCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn summary_foot(&self) -> SummaryFoot { self.summary_foot.clone() }

        fn group_title(&self, id: &u32) -> Option<Span<'static>> {
            Some(Span::raw(group_title(*id)))
        }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            TileDemands {
                summary:       1,
                summary_width: 0,
                groups:        self
                    .groups
                    .iter()
                    .map(|&id| TileDemand { id, rows: 1 })
                    .collect(),
            }
        }

        fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
            let text = match content {
                TileContent::Summary => "table body",
                TileContent::Group(_) => "group body",
                TileContent::Empty(_) => return,
            };
            Paragraph::new(text).render(inner, buffer);
        }
    }

    /// One row of `buffer` as text, with the blanks to the right of it
    /// trimmed off.
    fn buffer_line(buffer: &Buffer, y: u16) -> String {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        line.trim_end().to_string()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| buffer_line(buffer, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The text inside `area`, one buffer row at a time.
    fn area_lines(buffer: &Buffer, area: Rect) -> Vec<String> {
        (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    /// The one-cell title used by motion fixtures, which stays whole
    /// until a closing column has no title cell left.
    fn group_title(id: u32) -> String {
        let index = usize::try_from(id).unwrap_or_default() % GROUP_TITLE_GLYPHS.len();
        char::from(GROUP_TITLE_GLYPHS[index]).to_string()
    }

    fn is_lattice_glyph(symbol: &str) -> bool {
        [
            line::HORIZONTAL,
            line::VERTICAL,
            line::TOP_LEFT,
            line::TOP_RIGHT,
            line::BOTTOM_LEFT,
            line::BOTTOM_RIGHT,
            line::VERTICAL_RIGHT,
            line::VERTICAL_LEFT,
            line::HORIZONTAL_DOWN,
            line::HORIZONTAL_UP,
            line::CROSS,
        ]
        .contains(&symbol)
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

    /// Draw hand-built moving pieces inside hand-built column frames.
    fn draw_motion_fixture(area: Rect, drawing: &TileDrawing<u32>) -> Buffer {
        draw_motion_fixture_with_width(area, drawing, area.width.saturating_sub(2))
    }

    /// One hand-built piece and its inseparable name role.
    fn tile_piece(content: TileContent<u32>, frame: PaneFrame, name: PieceName) -> TilePiece<u32> {
        TilePiece {
            placement: TilePlacement { content, frame },
            name,
        }
    }

    /// Draw moving pieces with the width their contents were measured at.
    fn draw_motion_fixture_with_width(
        area: Rect,
        drawing: &TileDrawing<u32>,
        measured: u16,
    ) -> Buffer {
        let groups = (7..=18).collect::<Vec<_>>();
        let demands = TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        groups
                .iter()
                .map(|&id| TileDemand { id, rows: 1 })
                .collect(),
        };
        let widths = groups
            .iter()
            .map(|&id| (TileContent::Group(id), measured))
            .collect::<Vec<_>>();
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups,
        };
        let mut buffer = Buffer::empty(area);
        draw_placements(
            &mut buffer,
            area,
            drawing,
            &demands,
            &widths,
            TileGridContents::Shown,
            &cells,
        );
        buffer
    }

    /// A full three-column grid opening a fourth column with three pieces.
    fn column_transition(area: Rect, opening: bool) -> (TileGrid<u32>, TileGrowth) {
        let growth = TileGrowth {
            initial_rows:  3,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let groups = |ids: &[u32]| TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
        };
        let (before, after): (&[u32], &[u32]) = if opening {
            (
                &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
                &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18],
            )
        } else {
            (
                &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18],
                &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
            )
        };
        let mut grid = TileGrid::new();
        grid.set_layout(area, growth);
        grid.sync(&groups(before), growth);
        grid.settle_for_test();
        grid.sync(&groups(after), growth);
        (grid, growth)
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

    /// Twenty-four evenly spaced readings of the raw motion clock.
    fn motion_snapshots() -> impl Iterator<Item = u32> {
        (0..MOTION_SNAPSHOTS).map(|step| step * PROGRESS_SCALE / (MOTION_SNAPSHOTS - 1))
    }

    /// The visible pane-interior rows after its shift and clip are applied.
    fn visible_body_rows(frame: PaneFrame) -> Option<Range<u16>> {
        let body = frame.inner();
        let clip = frame_inner(frame.clip());
        let top = (i64::from(body.top()) + i64::from(frame.shift())).max(i64::from(clip.top()));
        let bottom =
            (i64::from(body.bottom()) + i64::from(frame.shift())).min(i64::from(clip.bottom()));
        let top = u16::try_from(top).ok()?;
        let bottom = u16::try_from(bottom).ok()?;
        (top < bottom).then_some(top..bottom)
    }

    fn doubled_line_failure(buffer: &Buffer, area: Rect, motion: &str, raw: u32) -> Option<String> {
        for y in area.top()..area.bottom().saturating_sub(1) {
            for x in area.left()..area.right().saturating_sub(2) {
                let horizontal_run = |row| {
                    (x..x + 3).all(|column| buffer[(column, row)].symbol() == line::HORIZONTAL)
                };
                if horizontal_run(y) && horizontal_run(y + 1) {
                    return Some(format!(
                        "{motion} at raw {raw} draws horizontal lines on rows {y} and {}",
                        y + 1
                    ));
                }
            }
        }

        let right_edges = [
            line::VERTICAL,
            line::VERTICAL_LEFT,
            line::TOP_RIGHT,
            line::BOTTOM_RIGHT,
        ];
        let left_edges = [
            line::VERTICAL,
            line::VERTICAL_RIGHT,
            line::TOP_LEFT,
            line::BOTTOM_LEFT,
        ];
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right().saturating_sub(1) {
                let left = buffer[(x, y)].symbol();
                let right = buffer[(x + 1, y)].symbol();
                if right_edges.contains(&left) && left_edges.contains(&right) {
                    return Some(format!(
                        "{motion} at raw {raw} draws vertical lines side by side at ({x}, {y})"
                    ));
                }
            }
        }
        None
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

    /// Whether pane ground, a lattice line or an owned title covers one row.
    fn row_has_coverage(
        buffer: &Buffer,
        drawing: &TileDrawing<u32>,
        ground: Color,
        cells: Range<u16>,
        y: u16,
    ) -> bool {
        cells.into_iter().any(|x| {
            let cell = &buffer[(x, y)];
            cell.bg != ground
                || is_lattice_glyph(cell.symbol())
                || drawing
                    .pieces
                    .iter()
                    .any(|piece| piece_title_covers(piece, cell.symbol(), x, y))
        })
    }

    /// Whether this cell contains the exact title character this piece owns.
    fn piece_title_covers(piece: &TilePiece<u32>, symbol: &str, x: u16, y: u16) -> bool {
        if piece.name != PieceName::Shown {
            return false;
        }
        let clip = piece.placement.frame.clip();
        let Some(offset) = x.checked_sub(clip.left().saturating_add(1)) else {
            return false;
        };
        if y != clip.top() || x >= clip.right().saturating_sub(1) {
            return false;
        }
        let title = match &piece.placement.content {
            TileContent::Summary => "summary".to_string(),
            TileContent::Group(id) => group_title(*id),
            TileContent::Empty(_) => return false,
        };
        title
            .chars()
            .nth(usize::from(offset))
            .is_some_and(|character| {
                let mut encoded = [0; 4];
                symbol == character.encode_utf8(&mut encoded)
            })
    }

    /// A band row with no painted cell, frame line or title.
    fn bare_band_row_failure(
        buffer: &Buffer,
        drawing: &TileDrawing<u32>,
        motion: &str,
        raw: u32,
    ) -> Option<String> {
        let ground = screen_ground().unwrap_or(Color::Reset);
        for &band in &drawing.column_bands {
            let inner = frame_inner(band);
            if inner.is_empty() {
                continue;
            }
            for y in inner.top()..inner.bottom() {
                if !row_has_coverage(buffer, drawing, ground, inner.left()..inner.right(), y) {
                    let frames = drawing
                        .placements()
                        .filter(|placement| {
                            let clip = placement.frame.clip();
                            clip.x == band.x && clip.width == band.width
                        })
                        .map(|placement| {
                            (
                                &placement.content,
                                placement.frame.rect(),
                                placement.frame.shift(),
                                placement.frame.clip(),
                            )
                        })
                        .collect::<Vec<_>>();
                    return Some(format!(
                        "{motion} at raw {raw} leaves band {band:?} row {y} on the screen ground; \
                         frames={frames:?}"
                    ));
                }
            }
        }
        None
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

    /// Draw a settled grid with `cells` into a new buffer.
    fn settled_grid(area: Rect, cells: &StubCells) -> (TileGrid<u32>, Buffer) {
        let growth = TileGrowth::default();
        let mut grid = TileGrid::new();
        let mut buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
        grid.settle_for_test();
        buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
        (grid, buffer)
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
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       vec![7],
        };
        let mut buffer = Buffer::empty(area);

        draw_placements(
            &mut buffer,
            area,
            &drawing,
            &demands,
            &widths,
            TileGridContents::Shown,
            &cells,
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
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       vec![7, 8],
        };
        let draw = |contents| {
            let mut buffer = Buffer::empty(area);
            draw_placements(
                &mut buffer,
                area,
                &drawing,
                &demands,
                &widths,
                contents,
                &cells,
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

    fn title_demands(ids: &[u32]) -> TileDemands<u32> {
        TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
        }
    }

    fn title_motion(
        area: Rect,
        before: &[u32],
        after: &[u32],
        growth: TileGrowth,
    ) -> TileGrid<u32> {
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
        let grid =
            collapsed_title_motion(TITLE_AREA, &before, &[20, 22, 21, 23, 24], CLOSING_GROWTH);
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
                        .filter(|piece| {
                            piece.placement.content == TileContent::Group(departing)
                        })
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
    fn summary_foot_is_written_only_on_summary_readout_row() {
        let area = Rect::new(0, 0, 80, 16);
        let cells = StubCells {
            summary_foot: SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            groups:       vec![7],
        };
        let (grid, buffer) = settled_grid(area, &cells);
        let placements = grid.placements(area, TileGrowth::default());
        assert_eq!(placements.len(), 2);
        let summary = &placements[0];
        let group = &placements[1];
        assert_eq!(summary.content, TileContent::Summary);
        assert_eq!(group.content, TileContent::Group(7));
        let summary = summary.frame.inner();
        let group = group.frame.inner();

        let foot_x = summary.x + TILE_FOOT_LEFT_INSET;
        let foot_y = summary.bottom() - TILE_ROWS_READOUT_HEIGHT;
        let foot_width = u16::try_from(FOOT_TEXT.len()).unwrap_or(u16::MAX);
        let rendered: String = (foot_x..foot_x + foot_width)
            .map(|x| buffer[(x, foot_y)].symbol())
            .collect();
        assert_eq!(rendered, FOOT_TEXT);
        assert!(
            area_lines(&buffer, group)
                .iter()
                .all(|line| !line.contains(FOOT_TEXT)),
            "the group cell has no summary foot"
        );
    }

    #[test]
    fn summary_foot_leaves_wide_readout_against_right_inset() {
        let buffer_area = Rect::new(0, 0, 100, 8);
        let inner = Rect::new(3, 2, 80, 4);
        let mut buffer = Buffer::empty(buffer_area);
        draw_cell(
            &mut buffer,
            inner,
            1,
            inner.width,
            &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            |_, _| {},
        );

        let y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
        assert_eq!(
            buffer[(inner.right() - TILE_ROWS_RIGHT_INSET - 1, y)].symbol(),
            "0"
        );
        assert!(buffer_line(&buffer, y).contains(TILE_ROWS_CONTENT_LABEL));
    }

    #[test]
    fn summary_foot_shares_with_the_short_readout_candidate() {
        let inner = Rect::new(0, 0, 30, 3);
        let mut buffer = Buffer::empty(inner);
        draw_cell(
            &mut buffer,
            inner,
            1,
            inner.width,
            &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            |_, _| {},
        );

        let row = buffer_line(&buffer, inner.bottom() - TILE_ROWS_READOUT_HEIGHT);
        assert!(row.contains(FOOT_TEXT));
        assert!(row.contains(TILE_ROWS_CONTENT_LABEL));
        assert!(!row.contains(TILE_ROWS_CELL_LABEL));
    }

    #[test]
    fn summary_foot_is_drawn_whole_or_absent_at_every_width() {
        let foot_width = u16::try_from(Line::raw(FOOT_TEXT).width()).unwrap_or(u16::MAX);
        let lines_fit_side_by_side = |width| {
            let inner = Rect::new(0, 0, width, 3);
            let readout_width = rows_readout_lines(inner, 1, width)
                .last()
                .map_or(u16::MAX, |line| {
                    u16::try_from(line.width()).unwrap_or(u16::MAX)
                });
            foot_width
                .saturating_add(TILE_FOOT_LEFT_INSET)
                .saturating_add(TILE_FOOT_GAP)
                .saturating_add(readout_width)
                .saturating_add(TILE_ROWS_RIGHT_INSET)
                <= width
        };
        let first_side_by_side_width = (0..=u16::MAX)
            .find(|&width| lines_fit_side_by_side(width))
            .unwrap_or(u16::MAX);
        assert!(
            lines_fit_side_by_side(first_side_by_side_width),
            "the summary foot and readout fit side by side at some width"
        );

        for width in 0..=first_side_by_side_width {
            let inner = Rect::new(0, 0, width, 3);
            let y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
            let mut foot_buffer = Buffer::empty(inner);
            draw_cell(
                &mut foot_buffer,
                inner,
                1,
                width,
                &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
                |_, _| {},
            );

            if foot_width <= width.saturating_sub(TILE_FOOT_LEFT_INSET)
                && rows_readout_height(width) > 0
            {
                let rendered: String = (TILE_FOOT_LEFT_INSET..TILE_FOOT_LEFT_INSET + foot_width)
                    .map(|x| foot_buffer[(x, y)].symbol())
                    .collect();
                assert_eq!(rendered, FOOT_TEXT, "summary inner width {width}");
            } else {
                let mut empty_buffer = Buffer::empty(inner);
                draw_cell(
                    &mut empty_buffer,
                    inner,
                    1,
                    width,
                    &SummaryFoot::Empty,
                    |_, _| {},
                );
                for x in 0..width {
                    assert_eq!(
                        foot_buffer[(x, y)],
                        empty_buffer[(x, y)],
                        "summary inner width {width}, column {x}"
                    );
                }
            }
        }

        let inner = Rect::new(0, 0, first_side_by_side_width, 3);
        let mut buffer = Buffer::empty(inner);
        draw_cell(
            &mut buffer,
            inner,
            1,
            inner.width,
            &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            |_, _| {},
        );
        let row = buffer_line(&buffer, inner.bottom() - TILE_ROWS_READOUT_HEIGHT);
        assert!(row.contains(FOOT_TEXT));
        assert!(row.contains(TILE_ROWS_CONTENT_LABEL));
    }

    #[test]
    fn draw_tile_cell_draws_only_the_readout_on_its_foot_row() {
        let inner = Rect::new(0, 0, 50, 4);
        let mut buffer = Buffer::empty(inner);
        draw_tile_cell(
            &mut buffer,
            &TileContent::<u32>::Summary,
            inner,
            1,
            inner.width,
            |buffer, area| Paragraph::new("body").render(area, buffer),
        );

        let readout = "content rows: 1  r/c: 4/50";
        let width = usize::from(inner.width - TILE_ROWS_RIGHT_INSET);
        assert_eq!(buffer_line(&buffer, 0), "body");
        assert_eq!(buffer_line(&buffer, 1), "");
        assert_eq!(buffer_line(&buffer, 2), "");
        assert_eq!(buffer_line(&buffer, 3), format!("{readout:>width$}"));
    }

    #[test]
    fn the_rows_readout_is_a_whole_candidate_at_every_width() {
        let measured_at = 17;
        for width in 0..=60 {
            let inner = Rect::new(0, 0, width, 4);
            let candidates = rows_readout_lines(inner, 22, measured_at)
                .iter()
                .map(line_text)
                .collect::<Vec<_>>();
            let mut buffer = Buffer::empty(inner);
            draw_rows_readout(&mut buffer, inner, 22, measured_at);
            let row = buffer_line(&buffer, inner.bottom() - 1);
            let shown = row.trim();
            assert!(
                shown.is_empty() || candidates.iter().any(|candidate| candidate == shown),
                "width {width} drew {shown:?}; candidates={candidates:?}"
            );
        }
    }

    #[test]
    fn a_cell_with_one_body_row_draws_its_contents_and_no_readout() {
        let inner = Rect::new(0, 0, 50, 1);
        let mut buffer = Buffer::empty(inner);
        draw_tile_cell(
            &mut buffer,
            &TileContent::<u32>::Summary,
            inner,
            1,
            inner.width,
            |buffer, area| Paragraph::new("body").render(area, buffer),
        );

        assert_eq!(buffer_line(&buffer, 0), "body");
        assert!(!buffer_line(&buffer, 0).contains(TILE_ROWS_CONTENT_LABEL));
        assert!(matches!(readout_row(inner), ReadoutRow::CellTooSmall));
    }

    #[test]
    fn a_grid_too_wide_for_its_window_draws_the_summary_alone() {
        let growth = TileGrowth {
            initial_rows:  1,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       (1..=8).collect(),
        };
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
        );
        grid.settle_for_test();

        let area = Rect::new(0, 0, 100, TITLE_AREA.height);
        let mut buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            &cells,
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
    fn the_cells_stand_settled_in_the_first_frame_that_holds_them() {
        let growth = TileGrowth {
            initial_rows:  1,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       (1..=8).collect(),
        };
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
        );

        buffer = Buffer::empty(TITLE_AREA);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            TITLE_AREA,
            growth,
            TileGridContents::Shown,
            &cells,
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
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       vec![1, 2],
        };
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
        );
        grid.settle_for_test();
        assert_eq!(grid.placements(large, growth).len(), 3);
    }

    #[test]
    fn a_command_refused_for_height_opens_when_there_is_room() {
        let growth = TileGrowth::default();
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       vec![1, 2],
        };
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
        );
        grid.settle_for_test();
        assert_eq!(grid.placements(short, growth).len(), 1);

        let cell_count = u16::try_from(cells.groups.len().saturating_add(1)).unwrap_or(u16::MAX);
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
        );
        grid.settle_for_test();
        assert_eq!(grid.placements(tall, growth).len(), 3);
    }

    #[test]
    fn a_hidden_grid_too_small_draws_nothing() {
        let area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TITLE_AREA.height);
        let cells = StubCells {
            summary_foot: SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            groups:       vec![1],
        };
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
        );

        assert_eq!(buffer_text(&buffer).trim(), "");
    }

    #[test]
    fn a_closing_column_in_flight_never_draws_a_cell_under_the_floor() {
        let growth = TileGrowth {
            initial_rows:  1,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let before = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       (1..=8).collect(),
        };
        let after = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       (1..=3).collect(),
        };
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
        );
        grid.settle_for_test();
        grid.collapse_queued_changes_for_test();
        let area = Rect::new(0, 0, 79, TITLE_AREA.height);
        buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            &after,
        );

        for raw in motion_snapshots() {
            assert!(!grid.drawing_at(area, growth, raw).pieces.is_empty());
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
        let area = Rect::new(0, 0, 79, TITLE_AREA.height);
        grid.set_layout(area, growth);
        grid.sync(
            &TileDemands {
                summary:       1,
                summary_width: 0,
                groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
            },
            growth,
        );
        let cells = StubCells {
            summary_foot: SummaryFoot::Empty,
            groups:       (1..=5).collect(),
        };
        let mut buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            &cells,
        );

        for raw in motion_snapshots() {
            assert!(!grid.drawing_at(area, growth, raw).pieces.is_empty());
            assert_eq!(grid.display(), GridDisplay::SummaryAlone);
        }
        assert!(!buffer_text(&buffer).contains("group body"));
    }

    #[test]
    fn hidden_grid_draws_no_summary_foot() {
        let area = Rect::new(0, 0, 80, 8);
        let cells = StubCells {
            summary_foot: SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            groups:       Vec::new(),
        };
        let mut grid = TileGrid::new();
        let mut buffer = Buffer::empty(area);
        draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            TileGrowth::default(),
            TileGridContents::Hidden,
            &cells,
        );

        assert!(
            area_lines(&buffer, area)
                .iter()
                .all(|line| !line.contains(FOOT_TEXT))
        );
    }

    #[test]
    fn an_empty_tile_number_uses_the_only_body_row_before_the_readout() {
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

            let sole_row = height == TILE_ROWS_READOUT_HEIGHT;
            assert_eq!(
                buffer_line(&buffer, height - 1).contains(TILE_ROWS_CONTENT_LABEL),
                !sole_row,
                "the readout needs a separate row"
            );
            assert!(
                buffer_line(&buffer, 0).contains(&number.to_string()),
                "the tile number keeps the first body row"
            );
        }
    }
}

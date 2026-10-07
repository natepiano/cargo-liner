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
use super::grid::TileContent;
use super::grid::TileDemands;
use super::grid::TileGrid;
use super::growth::TileGrowth;
use crate::GridLines;
use crate::PaneBorders;
use crate::PaneFrameLabel;
use crate::default_pane_chrome;
use crate::draw_clipped;
use crate::error_color;
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
    /// The app's text, drawn from the left inset.
    Text(Line<'static>),
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

    /// Text written at the left end of the summary cell's readout row.
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
    let widths = grid.content_widths(area, growth);
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    grid.sync(&demands, growth);
    let placements = grid.placements(area, growth);
    // Painted solid only while the work is shown: the attract screen
    // draws over bare panes, and keeps the terminal's own background.
    if contents == TileGridContents::Shown
        && let Some(ground) = screen_ground()
    {
        buffer.set_style(area, Style::default().bg(ground));
    }
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
            let summary_foot = match &placement.content {
                TileContent::Summary => cells.summary_foot(),
                TileContent::Group(_) | TileContent::Empty(_) => SummaryFoot::Empty,
            };
            draw_clipped(buffer, placement.frame, |buffer, inner| {
                draw_cell(
                    buffer,
                    &placement.content,
                    inner,
                    content_rows,
                    demand_width,
                    &summary_foot,
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
            // A group's title names what the cell holds, so it goes with
            // the contents.
            TileContent::Group(id) => match cells
                .group_title(id)
                .filter(|_| contents == TileGridContents::Shown)
            {
                Some(title) => grid_lines.add_titled(placement.frame, title),
                None => grid_lines.add(placement.frame),
            },
            TileContent::Empty(_) => {
                grid_lines.add(placement.frame);
            },
        }
    }
    // Neighbouring tiles meet on one line, so no cell belongs to a
    // single tile. Focus is the background tint under a tile's contents,
    // and lights the tile's border only where the screen is transparent
    // and no tint is painted.
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
    draw_cell(
        buffer,
        content,
        inner,
        rows,
        measured_at,
        &SummaryFoot::Empty,
        draw,
    );
}

/// Draw one cell with the app's summary foot sharing its readout row.
fn draw_cell<Id>(
    buffer: &mut Buffer,
    content: &TileContent<Id>,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    summary_foot: &SummaryFoot,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    let contents = content_area(inner);
    if !contents.is_empty() {
        match content {
            TileContent::Empty(number) => draw_number(buffer, *number, contents),
            TileContent::Summary | TileContent::Group(_) => draw(buffer, contents),
        }
    }
    match summary_foot {
        SummaryFoot::Empty => draw_rows_readout(buffer, inner, rows, measured_at),
        SummaryFoot::Text(line) => {
            let foot_width = draw_summary_foot(buffer, inner, line);
            draw_rows_readout_after_foot(buffer, inner, rows, measured_at, foot_width);
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
    let line = rows_readout_line(inner, rows, measured_at);
    draw_rows_readout_line(buffer, inner, line);
}

/// Draw the rows readout only when its whole line fits after the summary foot.
fn draw_rows_readout_after_foot(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    foot_width: u16,
) {
    let line = rows_readout_line(inner, rows, measured_at);
    let line_width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    let room = inner
        .width
        .saturating_sub(TILE_ROWS_RIGHT_INSET)
        .saturating_sub(TILE_FOOT_LEFT_INSET)
        .saturating_sub(foot_width)
        .saturating_sub(TILE_FOOT_GAP);
    if line_width <= room {
        draw_rows_readout_line(buffer, inner, line);
    }
}

/// Build the styled readout that reports content and cell dimensions.
fn rows_readout_line(inner: Rect, rows: usize, measured_at: u16) -> Line<'static> {
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
    Line::from(spans)
}

/// Right-align one rows readout, clipping it to the cell when needed.
fn draw_rows_readout_line(buffer: &mut Buffer, inner: Rect, line: Line<'static>) {
    let Some(area) = readout_area(inner, u16::try_from(line.width()).unwrap_or(u16::MAX)) else {
        return;
    };
    Paragraph::new(line).render(area, buffer);
}

/// Draw the app's text at the left end of the readout row.
fn draw_summary_foot(buffer: &mut Buffer, inner: Rect, line: &Line<'static>) -> u16 {
    let height = rows_readout_height(inner.width);
    if height == 0 || inner.height < TILE_ROWS_READOUT_HEIGHT {
        return 0;
    }
    let width = u16::try_from(line.width())
        .unwrap_or(u16::MAX)
        .min(inner.width.saturating_sub(TILE_FOOT_LEFT_INSET));
    Paragraph::new(line.clone()).render(
        Rect {
            x: inner.x.saturating_add(TILE_FOOT_LEFT_INSET),
            y: inner.bottom().saturating_sub(TILE_ROWS_READOUT_HEIGHT),
            width,
            height,
        },
        buffer,
    );
    width
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
    use crate::TileDemand;

    const FOOT_TEXT: &str = "mem 2.5G";

    struct StubCells {
        summary_foot: SummaryFoot,
        groups:       Vec<u32>,
    }

    impl TileCells<u32> for StubCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn summary_foot(&self) -> SummaryFoot { self.summary_foot.clone() }

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
                TileContent::Summary => "summary body",
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
            &TileContent::<u32>::Summary,
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
    fn summary_foot_wins_when_readout_does_not_fit() {
        let inner = Rect::new(0, 0, 30, 3);
        let mut buffer = Buffer::empty(inner);
        draw_cell(
            &mut buffer,
            &TileContent::<u32>::Summary,
            inner,
            1,
            inner.width,
            &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            |_, _| {},
        );

        let row = buffer_line(&buffer, inner.bottom() - TILE_ROWS_READOUT_HEIGHT);
        assert!(row.contains(FOOT_TEXT));
        assert!(!row.contains(TILE_ROWS_CONTENT_LABEL));
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

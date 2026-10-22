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
use crate::blend_color;
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

/// How far a cell's ink has travelled toward its ground.
/// Zero retains the ink; [`u8::MAX`] reaches the ground.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellFadeProgress(u8);

impl CellFadeProgress {
    /// Name an amount on the same scale as [`crate::blend_color`].
    #[must_use]
    pub const fn new(amount: u8) -> Self { Self(amount) }

    /// The amount on the zero-to-255 blend scale.
    #[must_use]
    pub const fn amount(self) -> u8 { self.0 }
}

/// Whether a cell's foot follows its contents toward the cell's ground.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CellReadoutFade {
    /// Retain the foot's original inks.
    #[default]
    Unfaded,
    /// Blend the foot's inks toward the cell's ground.
    Fading(CellFadeProgress),
}

/// Whether the measured rows include the cell's complete source content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CellContentExtent {
    /// All source content participates in the row measurement.
    #[default]
    Measured,
    /// The source omitted content before its known rows could be measured.
    CutAtSource,
}

/// Ink state shared by every span of a cell's foot.
#[derive(Clone, Copy)]
enum CellReadoutInk {
    Unfaded,
    Fading {
        progress: CellFadeProgress,
        ground:   Color,
    },
}

impl CellReadoutInk {
    fn line(self, mut line: Line<'static>) -> Line<'static> {
        let Self::Fading { progress, ground } = self else {
            return line;
        };
        for span in &mut line.spans {
            let ink = line.style.patch(span.style).fg.unwrap_or_else(text_default);
            span.style = span.style.fg(blend_color(ink, ground, progress.amount()));
        }
        line
    }
}

/// Source completeness and fading for one cell's readout.
struct CellReadout {
    extent: CellContentExtent,
    ink:    CellReadoutInk,
}

impl From<CellReadoutInk> for CellReadout {
    fn from(ink: CellReadoutInk) -> Self {
        Self {
            extent: CellContentExtent::Measured,
            ink,
        }
    }
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

    /// Completeness of the source, independent of the cell's measured row demand.
    fn content_extent(&self, _content: &TileContent<Id>) -> CellContentExtent {
        CellContentExtent::Measured
    }

    /// Measured rows shown in the foot at this allocation, excluding surrendered decoration.
    /// The allocation and motion continue to use the original demand.
    fn readout_rows(&self, _content: &TileContent<Id>, _area: Rect, measured_rows: usize) -> usize {
        measured_rows
    }

    /// Text drawn whole or not at all at the left end of the summary
    /// cell's readout row.
    fn summary_foot(&self) -> SummaryFoot { SummaryFoot::Empty }

    /// Fade used by the cell's contents, also applied to every foot span.
    /// Existing cell providers retain their original readout inks.
    fn readout_fade(&self, _content: &TileContent<Id>) -> CellReadoutFade {
        CellReadoutFade::Unfaded
    }

    /// Set frame-wide content widths before demand and allocation measurements.
    /// Call `allocations` only when preparation needs the current allocation layout.
    /// Areas exclude borders and foot rows, using the same calculation as painting.
    fn prepare_frame(&self, _allocations: impl FnOnce() -> Vec<(TileContent<Id>, Rect)>) {}

    /// Rows each cell's contents would take given all the room they
    /// want, each measured at the width `widths` gives that cell.
    ///
    /// Content rows only: the grid adds the readout row itself. A group
    /// `widths` does not carry yet -- one first seen on this frame --
    /// belongs at the narrowest width in `widths`, which is the width it
    /// will have once the grid has opened a cell for it.
    fn demands(&self, widths: &[(TileContent<Id>, u16)]) -> TileDemands<Id>;

    /// Rows the cell's content paints when `area` is the room available.
    ///
    /// The grid reads this answer when checking whether a moving piece
    /// fills its allocation. Content that can paint every row keeps the
    /// default.
    fn rows_drawn(&self, _content: &TileContent<Id>, area: Rect) -> u16 { area.height }

    /// Rows the grid must leave with the cell before its column mates may
    /// take the rest.
    ///
    /// Most content retains every row it paints. Content with an optional
    /// part may return less here so a mate can complete its next row step.
    fn rows_kept(&self, content: &TileContent<Id>, area: Rect) -> u16 {
        self.rows_drawn(content, area)
    }

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
    /// The line's and each span's styles are patched over the chrome's
    /// title style, so a title that sets only a colour keeps the weight
    /// focus gives it, and an unstyled one is drawn as the summary's
    /// title is.
    ///
    /// [`summary_title`]: Self::summary_title
    fn group_title(&self, _id: &Id) -> Option<Line<'static>> { None }
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
    grid.set_draw_layout(area, growth);
    cells.prepare_frame(|| grid.preparation_allocations(area, growth));
    let cells_widths = measurement_widths(grid, area, growth, GridDisplay::Cells);
    let mut cells_demands = cells.demands(&cells_widths);
    let alone_widths = measurement_widths(grid, area, growth, GridDisplay::SummaryAlone);
    let mut alone_demands = cells.demands(&alone_widths);
    add_readout_rows(&mut cells_demands, &cells_widths);
    add_readout_rows(&mut alone_demands, &alone_widths);
    grid.sync_rendered_demands(&cells_demands, growth);
    let progress = grid.progress();
    grid.measure_row_steps(
        area,
        growth,
        progress,
        |content, area| cells.rows_drawn(content, area),
        |content, area| cells.rows_kept(content, area),
    );
    let display = grid.display();
    if display == GridDisplay::SummaryAlone
        && (contents == TileGridContents::Hidden || frame_inner(area).is_empty())
    {
        grid.record_painted_summary_frames(Vec::new());
        return;
    }
    let (widths, demands) = match display {
        GridDisplay::Cells => (&cells_widths, &cells_demands),
        GridDisplay::SummaryAlone => (&alone_widths, &alone_demands),
    };
    let drawing = grid.drawing_at(area, growth, progress);
    let frames = draw_placements(buffer, area, &drawing, demands, widths, contents, cells);
    grid.record_painted_summary_frames(frames);
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
pub(super) fn draw_placements<Id: Clone + Eq + Debug>(
    buffer: &mut Buffer,
    area: Rect,
    drawing: &TileDrawing<Id>,
    demands: &TileDemands<Id>,
    widths: &[(TileContent<Id>, u16)],
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
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
        let content_rows = cell_rows.saturating_sub(usize::from(rows_readout_height(demand_width)));
        if contents == TileGridContents::Shown {
            let summary_foot = match &placement.content {
                TileContent::Summary => cells.summary_foot(),
                TileContent::Group(_) | TileContent::Empty(_) => SummaryFoot::Empty,
            };
            draw_clipped(buffer, placement.frame, |buffer, inner| {
                draw_cell(
                    buffer,
                    inner,
                    cells.readout_rows(&placement.content, content_area(inner), content_rows),
                    demand_width,
                    &summary_foot,
                    CellReadout {
                        extent: cells.content_extent(&placement.content),
                        ink:    match cells.readout_fade(&placement.content) {
                            CellReadoutFade::Unfaded => CellReadoutInk::Unfaded,
                            CellReadoutFade::Fading(progress) => {
                                CellReadoutInk::Fading { progress, ground }
                            },
                        },
                    },
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
    let chrome = default_pane_chrome();
    let chrome = painted_ground.map_or(chrome, |ground| PaneChrome {
        inactive_border: chrome.inactive_border.bg(ground),
        ..chrome
    });
    grid_lines.render(buffer, chrome, PaneBorders::Shared);
    painted_summary_frames
}

/// Whether a placement has a pane-interior row visible between its
/// clip's frame rows.
pub(super) fn has_visible_body_row(frame: PaneFrame) -> bool {
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
        CellReadoutInk::Unfaded,
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
    readout: impl Into<CellReadout>,
    draw: impl FnOnce(&mut Buffer, Rect),
) {
    let CellReadout { extent, ink } = readout.into();
    let contents = content_area(inner);
    if !contents.is_empty() {
        draw(buffer, contents);
    }
    let foot_draw_outcome = match summary_foot {
        SummaryFoot::Empty => SummaryFootDrawOutcome::NotDrawn,
        SummaryFoot::Text(line) => draw_summary_foot(buffer, inner, line, ink),
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
fn draw_rows_readout(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    extent: CellContentExtent,
    ink: CellReadoutInk,
) {
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return;
    };
    if let Some(line) = extent_readout_lines(inner, rows, measured_at, extent)
        .into_iter()
        .find(|line| line.width() <= usize::from(readout.width))
    {
        draw_rows_readout_line(buffer, readout, ink.line(line));
    }
}

/// Draw the rows readout only when its whole line fits after the summary foot.
fn draw_rows_readout_after_foot(
    buffer: &mut Buffer,
    inner: Rect,
    rows: usize,
    measured_at: u16,
    foot_width: u16,
    extent: CellContentExtent,
    ink: CellReadoutInk,
) {
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return;
    };
    let room = readout
        .width
        .saturating_sub(TILE_FOOT_LEFT_INSET)
        .saturating_sub(foot_width)
        .saturating_sub(TILE_FOOT_GAP);
    if let Some(line) = extent_readout_lines(inner, rows, measured_at, extent)
        .into_iter()
        .find(|line| line.width() <= usize::from(room))
    {
        draw_rows_readout_line(buffer, readout, ink.line(line));
    }
}

/// Build the styled readouts that fit progressively narrower cells.
fn extent_readout_lines(
    inner: Rect,
    rows: usize,
    measured_at: u16,
    extent: CellContentExtent,
) -> Vec<Line<'static>> {
    let reading = if extent == CellContentExtent::Measured
        && rows <= usize::from(content_area(inner).height)
    {
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

#[cfg(test)]
fn rows_readout_lines(inner: Rect, rows: usize, measured_at: u16) -> Vec<Line<'static>> {
    extent_readout_lines(inner, rows, measured_at, CellContentExtent::Measured)
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
    ink: CellReadoutInk,
) -> SummaryFootDrawOutcome {
    let ReadoutRow::Reserved(readout) = readout_row(inner) else {
        return SummaryFootDrawOutcome::NotDrawn;
    };
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    if width > inner.width.saturating_sub(TILE_FOOT_LEFT_INSET) {
        return SummaryFootDrawOutcome::NotDrawn;
    }
    Paragraph::new(ink.line(line.clone())).render(
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
pub(super) const fn content_area(inner: Rect) -> Rect {
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
    use std::cell::RefCell;
    use std::ops::Range;

    use ratatui::symbols::line;

    use super::*;
    use crate::FRAME_POLL_MILLIS;
    use crate::PaneFocusState;
    use crate::TileAction;
    use crate::TileDemand;
    use crate::TileFill;
    use crate::TileView;
    use crate::ToastActivation;
    use crate::ToastStyle;
    use crate::tiles::constants::COLUMN_FOOT_TEST_AREA;
    use crate::tiles::constants::COLUMN_FOOT_TEST_BODY_STEP;
    use crate::tiles::constants::COLUMN_FOOT_TEST_GROUP_COUNT;
    use crate::tiles::constants::COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS;
    use crate::tiles::constants::COLUMN_FOOT_TEST_MIN_HEIGHT;
    use crate::tiles::constants::COLUMN_FOOT_TEST_SUMMARY_ROWS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_COLUMN_ENTRIES;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_COLUMN_ROWS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_COMMAND_ROWS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_EMPTY_CELLS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_ENTRIES;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_GROUPS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_PROGRESS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_PROJECT_ROWS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_ROW;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_SOURCE_SPANS;
    use crate::tiles::constants::CONTENT_CHANGE_TEST_SPARE_SPANS;
    use crate::tiles::constants::DEPARTED_ENTRY_TEST_SPANS;
    use crate::tiles::constants::EMPTY_DONOR_TEST_SPANS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_GROUPS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_HEIGHT;
    use crate::tiles::constants::LARGE_CONTENT_TEST_INITIAL_ROWS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_MEASUREMENT_BOUND;
    use crate::tiles::constants::LARGE_CONTENT_TEST_ROW_STEP;
    use crate::tiles::constants::LARGE_CONTENT_TEST_ROWS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_WIDTHS;
    use crate::tiles::constants::LENDING_TEST_CUT_GROUP;
    use crate::tiles::constants::LENDING_TEST_DEMANDS;
    use crate::tiles::constants::LENDING_TEST_DRAWN;
    use crate::tiles::constants::LENDING_TEST_ENDPOINT_HEIGHTS;
    use crate::tiles::constants::LENDING_TEST_GAP_HEIGHT;
    use crate::tiles::constants::LENDING_TEST_INITIAL_ROWS;
    use crate::tiles::constants::LENDING_TEST_SUMMARY_TITLE;
    use crate::tiles::constants::LENDING_TEST_WHOLE_GROUP;
    use crate::tiles::constants::MIN_TILE_HEIGHT;
    use crate::tiles::constants::PROGRESS_SCALE;
    use crate::tiles::constants::SETTLED_SPARE_TEST_HEIGHTS;
    use crate::tiles::constants::SLIDING_TEST_INITIAL_ROWS;
    use crate::tiles::constants::SLIDING_TEST_INTERVALS;
    use crate::tiles::constants::SLIDING_TEST_MIN_CHANGES;
    use crate::tiles::constants::SLIDING_TEST_ROWS;
    use crate::tiles::constants::SUMMARY_DONOR_TEST_SPANS;
    use crate::tiles::constants::SUMMARY_STEP_TEST_ENTRY_ROWS;
    use crate::tiles::constants::SUMMARY_STEP_TEST_HEADER_ROWS;
    use crate::tiles::constants::SUMMARY_STEP_TEST_OPTIONAL_HEIGHTS;
    use crate::tiles::constants::SUMMARY_STEP_TEST_PIECES;
    use crate::tiles::constants::SUMMARY_STEP_TEST_ROWS;
    use crate::tiles::constants::TILE_ANIMATION_MILLIS;
    use crate::tiles::grid::PieceHeightSnap;
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
            &cells.demands(&widths),
            &widths,
            TileGridContents::Shown,
            &cells,
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
            &cells.demands(&widths),
            &widths,
            TileGridContents::Hidden,
            &cells,
        );
        assert_eq!(frames, []);
        Ok(())
    }

    struct StubCells {
        summary_foot:  SummaryFoot,
        demands:       TileDemands<u32>,
        row_counts:    Vec<StubRowCount>,
        readout_fades: Vec<(TileContent<u32>, CellReadoutFade)>,
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
        fn draw(
            &self,
            buffer: &mut Buffer,
            content: &TileContent<u32>,
            inner: Rect,
            ground: Color,
        ) {
            self.cells.draw(buffer, content, inner, ground);
        }
        fn content_extent(&self, _: &TileContent<u32>) -> CellContentExtent { self.extent }
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
                );
                let display = std::thread::scope(|readers| readers.spawn(|| grid.display()).join());
                assert!(matches!(display, Ok(GridDisplay::Cells)));
                assert!(!cells.queries.borrow().is_empty());
            }
        }
    }

    struct StubRowCount {
        content: TileContent<u32>,
        count:   u16,
    }

    impl StubCells {
        fn new(groups: impl IntoIterator<Item = u32>) -> Self {
            Self::demanding(1, groups.into_iter().map(|id| (id, 1)))
        }

        fn demanding(summary: usize, groups: impl IntoIterator<Item = (u32, usize)>) -> Self {
            Self {
                summary_foot:  SummaryFoot::Empty,
                demands:       TileDemands {
                    summary,
                    summary_width: 0,
                    groups: groups
                        .into_iter()
                        .map(|(id, rows)| TileDemand { id, rows })
                        .collect(),
                },
                row_counts:    Vec::new(),
                readout_fades: Vec::new(),
            }
        }

        fn with_summary_foot(mut self, summary_foot: SummaryFoot) -> Self {
            self.summary_foot = summary_foot;
            self
        }

        fn with_rows_drawn(mut self, content: TileContent<u32>, count: u16) -> Self {
            self.row_counts.push(StubRowCount { content, count });
            self
        }
    }

    impl TileCells<u32> for StubCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn summary_foot(&self) -> SummaryFoot { self.summary_foot.clone() }

        fn readout_fade(&self, content: &TileContent<u32>) -> CellReadoutFade {
            self.readout_fades
                .iter()
                .find(|(cell, _)| cell == content)
                .map_or(CellReadoutFade::Unfaded, |(_, fade)| *fade)
        }

        fn group_title(&self, id: &u32) -> Option<Line<'static>> {
            Some(Line::raw(group_title(*id)))
        }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            self.demands.clone()
        }

        fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
            self.row_counts
                .iter()
                .find(|count| &count.content == content)
                .map_or_else(
                    || {
                        let rows = match content {
                            TileContent::Summary => self.demands.summary,
                            TileContent::Group(id) => self
                                .demands
                                .groups
                                .iter()
                                .find(|group| &group.id == id)
                                .map_or(0, |group| group.rows),
                            TileContent::Empty(_) => 0,
                        };
                        u16::try_from(rows).unwrap_or(u16::MAX).min(area.height)
                    },
                    |count| count.count.min(area.height),
                )
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

    struct Toast64MotionCells {
        groups:       u32,
        summary_rows: usize,
    }

    impl TileCells<u32> for Toast64MotionCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            TileDemands {
                summary:       self.summary_rows,
                summary_width: 0,
                groups:        (1..=self.groups)
                    .map(|id| TileDemand { id, rows: 18 })
                    .collect(),
            }
        }

        fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
            // The grid reserves the foot row outside this content allocation.
            let drawn = match content {
                TileContent::Summary if area.width >= 80 => 13.min(area.height),
                TileContent::Summary if area.height == u16::MAX => 18,
                TileContent::Summary if area.height >= 18 => 18,
                TileContent::Summary if area.height >= 13 => 13,
                TileContent::Summary if area.height >= 9 => 9,
                TileContent::Summary if area.height >= 5 => 5,
                TileContent::Summary => area.height,
                TileContent::Group(_) => 18.min(area.height),
                TileContent::Empty(_) => 0,
            };
            match content {
                TileContent::Summary => {
                    drawn.min(u16::try_from(self.summary_rows).unwrap_or(u16::MAX))
                },
                TileContent::Group(_) | TileContent::Empty(_) => drawn,
            }
        }

        fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
            draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
        }
    }

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

    fn draw_stub_rows(buffer: &mut Buffer, area: Rect, rows: u16) {
        if area.width == 0 {
            return;
        }
        for y in area.top()..area.top().saturating_add(rows.min(area.height)) {
            buffer[(area.x, y)].set_symbol("x");
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
            height_snap: PieceHeightSnap::Settled,
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
        let cells = StubCells::new(groups);
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

    /// Draw twice around settling, with a selected view rule.
    fn settled_grid_in_view(
        area: Rect,
        cells: &StubCells,
        view: TileView,
        min_tile_height: u16,
    ) -> (TileGrid<u32>, Buffer) {
        let growth = TileGrowth::default();
        let mut grid = TileGrid::new();
        grid.set_view(view);
        grid.set_min_tile_height(min_tile_height);
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

    /// Resolve one exact motion point through the production measurement path.
    fn fixed_motion_drawing(
        grid: &mut TileGrid<u32>,
        cells: &impl TileCells<u32>,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
    ) -> TileDrawing<u32> {
        grid.set_layout(area, growth);
        let cells_widths = measurement_widths(grid, area, growth, GridDisplay::Cells);
        let mut cells_demands = cells.demands(&cells_widths);
        let alone_widths = measurement_widths(grid, area, growth, GridDisplay::SummaryAlone);
        let mut alone_demands = cells.demands(&alone_widths);
        add_readout_rows(&mut cells_demands, &cells_widths);
        add_readout_rows(&mut alone_demands, &alone_widths);
        grid.sync_rendered_demands(&cells_demands, growth);
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| cells.rows_drawn(content, area),
            |content, area| cells.rows_kept(content, area),
        );
        grid.drawing_at(area, growth, raw)
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
        for raw in 0..=PROGRESS_SCALE {
            let drawing = fixed_motion_drawing(&mut grid, &cells, area, growth, raw);
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

    /// Whether one placement uses all its rows or shows its whole value.
    fn placement_fits(cells: &impl TileCells<u32>, placement: &TilePlacement<u32>) -> bool {
        let area = content_area(placement.frame.inner());
        let drawn = cells.rows_drawn(&placement.content, area);
        let whole = cells.rows_drawn(
            &placement.content,
            Rect {
                height: u16::MAX,
                ..area
            },
        );
        drawn == area.height || drawn >= whole
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
        );
        grid.settle_for_test();
        grid.collapse_queued_changes_for_test();

        let mut mates_hide_rows = false;
        // Sample the runner's 8 ms FRAME_POLL_MILLIS stride across the default
        // 720 ms TILE_ANIMATION_MILLIS motion, including its first and last frame.
        let frame_stride = usize::try_from(FRAME_POLL_MILLIS)?;
        let frame_progress = (0..TILE_ANIMATION_MILLIS)
            .step_by(frame_stride)
            .map(|elapsed| {
                u32::try_from(elapsed * u64::from(PROGRESS_SCALE) / TILE_ANIMATION_MILLIS)
            })
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
        );
        grid.settle_for_test();

        for raw in 1..PROGRESS_SCALE {
            let drawing = fixed_motion_drawing(&mut grid, &after, area, growth, raw);
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
            );

            assert_eq!(grid.display(), GridDisplay::SummaryAlone, "{view:?}");
            assert!(!buffer_text(&buffer).contains("group body"), "{view:?}");
        }
    }

    #[test]
    fn the_view_holds_through_a_motion() {
        let area = Rect::new(0, 0, 77, 40);
        let min_tile_height = 14;
        let growth = TileGrowth::default();
        let quiet = StubCells::demanding(1, (1..=5).map(|id| (id, 1)))
            .with_rows_drawn(TileContent::Summary, 1);
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
        let cells = StubCells::new([7, 8]);
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

    #[test]
    fn an_arriving_piece_draws_no_line_until_it_has_a_row() {
        let area = Rect::new(0, 0, 6, 5);
        let rect = Rect::new(0, 0, 6, 3);
        let start_shift = i64::from(frame_inner(area).bottom())
            .saturating_sub(i64::from(frame_inner(rect).top()));
        let title = group_title(7);
        let mut saw_edge_only = false;
        let mut saw_body = false;

        for raw in 0..=PROGRESS_SCALE {
            let shift = start_shift.saturating_sub(
                start_shift.saturating_mul(i64::from(raw)) / i64::from(PROGRESS_SCALE),
            );
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

    /// Draw every sampled frame with the row probes used by the renderer.
    fn assert_column_foot_has_no_doubled_line(
        grid: &mut TileGrid<u32>,
        cells: &impl TileCells<u32>,
        growth: TileGrowth,
    ) {
        for raw in motion_snapshots() {
            let drawing = fixed_motion_drawing(grid, cells, COLUMN_FOOT_TEST_AREA, growth, raw);
            let widths =
                measurement_widths(grid, COLUMN_FOOT_TEST_AREA, growth, GridDisplay::Cells);
            let mut demands = cells.demands(&widths);
            add_readout_rows(&mut demands, &widths);
            let mut buffer = Buffer::empty(COLUMN_FOOT_TEST_AREA);
            draw_placements(
                &mut buffer,
                COLUMN_FOOT_TEST_AREA,
                &drawing,
                &demands,
                &widths,
                TileGridContents::Shown,
                cells,
            );
            assert_eq!(
                doubled_line_failure(&buffer, COLUMN_FOOT_TEST_AREA, "column foot", raw),
                None,
                "frames={:?}",
                drawing
                    .placements()
                    .map(|placement| (&placement.content, placement.frame))
                    .collect::<Vec<_>>()
            );
        }
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

    /// A summary whose compact entries omit separators while commands stay whole.
    #[derive(Clone, Copy)]
    struct ChangingSummaryCells {
        entries: u16,
        groups:  u32,
    }

    impl TileCells<u32> for ChangingSummaryCells {
        fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            TileDemands {
                summary:       usize::from(self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS),
                summary_width: 0,
                groups:        (1..=self.groups)
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
                TileContent::Group(_) => CONTENT_CHANGE_TEST_COMMAND_ROWS.min(area.height),
                TileContent::Empty(_) => 0,
            }
        }

        fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
            draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
        }
    }

    /// Begin from a drawn settled column with the requested empty cells.
    fn changing_summary_grid(cells: ChangingSummaryCells, empties: usize) -> TileGrid<u32> {
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
        let growth = changing_summary_growth();
        fixed_motion_drawing(
            &mut grid,
            &cells,
            COLUMN_FOOT_TEST_AREA,
            growth,
            PROGRESS_SCALE,
        );
        grid.settle_for_test();
        for _ in 0..empties {
            fixed_motion_drawing(
                &mut grid,
                &cells,
                COLUMN_FOOT_TEST_AREA,
                growth,
                PROGRESS_SCALE,
            );
            grid.apply(TileAction::Add, growth);
            grid.settle_for_test();
        }
        fixed_motion_drawing(
            &mut grid,
            &cells,
            COLUMN_FOOT_TEST_AREA,
            growth,
            PROGRESS_SCALE,
        );
        grid
    }

    fn changing_summary_growth() -> TileGrowth {
        TileGrowth {
            initial_rows:  SLIDING_TEST_INITIAL_ROWS,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        }
    }

    /// Read the painted content back after measuring the exact motion frame.
    fn assert_motion_content_is_filled(
        grid: &mut TileGrid<u32>,
        cells: ChangingSummaryCells,
        raw: u32,
    ) -> TileDrawing<u32> {
        let area = COLUMN_FOOT_TEST_AREA;
        let drawing = fixed_motion_drawing(grid, &cells, area, changing_summary_growth(), raw);
        let mut buffer = Buffer::empty(area);
        let widths = measurement_widths(grid, area, changing_summary_growth(), GridDisplay::Cells);
        draw_placements(
            &mut buffer,
            area,
            &drawing,
            &cells.demands(&widths),
            &widths,
            TileGridContents::Shown,
            &cells,
        );
        assert_no_unusable_row(&cells, &drawing, 0, raw);
        for placement in drawing.placements() {
            let content = content_area(placement.frame.inner());
            let whole = cells.rows_drawn(
                &placement.content,
                Rect {
                    height: u16::MAX,
                    ..content
                },
            );
            if cells.rows_drawn(&placement.content, content) < whole && content.height > 0 {
                let row = buffer_line(&buffer, content.bottom().saturating_sub(1));
                assert!(row.contains(CONTENT_CHANGE_TEST_ROW), "{raw}: {row}");
            }
        }
        drawing
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
        let mut fresh =
            changing_summary_grid(cells, CONTENT_CHANGE_TEST_EMPTY_CELLS.saturating_sub(1));
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
    fn cached_widening_summary_grid(
        cells: &WideningSummaryCells,
        growth: TileGrowth,
    ) -> TileGrid<u32> {
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

    #[test]
    fn a_widening_summary_keeps_every_piece_inside_its_moving_band() {
        assert_width_retarget_tiles_bands(false);
    }

    #[test]
    fn a_narrowing_summary_keeps_every_piece_inside_its_moving_band() {
        assert_width_retarget_tiles_bands(true);
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
                &cells.demands(&widths),
                &widths,
                TileGridContents::Shown,
                &cells,
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

    fn assert_drawing_tiles_bands(drawing: &TileDrawing<u32>, raw: u32) {
        for placement in drawing.placements() {
            let clip = placement.frame.clip();
            if placement.content != TileContent::Summary {
                assert!(
                    drawing
                        .column_bands
                        .iter()
                        .any(|band| band.intersection(clip) == clip),
                    "{raw}: {:?} {clip:?} leaves {:?}",
                    placement.content,
                    drawing.column_bands
                );
            }
        }
        for band in &drawing.column_bands {
            let mut pieces = drawing
                .placements()
                .filter_map(|piece| {
                    let clip = piece.frame.clip();
                    if (clip.x == band.x && clip.width == band.width)
                        || (piece.content == TileContent::Summary
                            && clip.top() == band.top()
                            && clip.x == band.x)
                    {
                        Some(clip)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            pieces.sort_by_key(|rect| rect.top());
            assert_eq!(
                pieces.first().map(|rect| rect.top()),
                Some(band.top()),
                "{raw}: {pieces:?}, {band:?}"
            );
            assert_eq!(
                pieces.last().map(|rect| rect.bottom()),
                Some(band.bottom()),
                "{raw}: {pieces:?}, {band:?}"
            );
            for pair in pieces.windows(2) {
                assert_eq!(
                    pair[0].bottom().saturating_sub(1),
                    pair[1].top(),
                    "{raw}: {pair:?}"
                );
            }
        }
        let summary = drawing
            .placements()
            .find(|piece| piece.content == TileContent::Summary);
        for command in drawing
            .placements()
            .filter(|piece| piece.content != TileContent::Summary)
        {
            if let Some(summary) = &summary {
                let overlap = summary.frame.clip().intersection(command.frame.clip());
                assert!(
                    overlap.height <= 1 || overlap.width <= 1,
                    "{raw}: summary {:?} overlaps {:?} in {overlap:?}",
                    summary.frame.clip(),
                    command.frame.clip()
                );
            }
        }
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

    fn assert_no_givable_spare_row(
        cells: &impl TileCells<u32>,
        drawing: &TileDrawing<u32>,
        endpoint: &TileDrawing<u32>,
        raw: u32,
    ) {
        for piece in &drawing.pieces {
            if raw < PROGRESS_SCALE
                && matches!(
                    piece.height_snap,
                    PieceHeightSnap::Search { start: 1, .. }
                        | PieceHeightSnap::Search { end: 1, .. }
                )
            {
                continue;
            }
            let donor = &piece.placement;
            let area = content_area(donor.frame.inner());
            let kept = cells.rows_kept(&donor.content, area);
            let end = endpoint
                .placements()
                .find(|piece| piece.content == donor.content);
            let end_height = end.map_or(0, |piece| piece.frame.clip().height);
            let needs_row_at_endpoint = piece.height_snap != PieceHeightSnap::Settled
                && donor.frame.clip().height <= end_height;
            if kept >= area.height
                || donor.frame.clip().height <= COLUMN_FOOT_TEST_MIN_HEIGHT
                || needs_row_at_endpoint
            {
                continue;
            }
            for recipient in drawing
                .pieces
                .iter()
                .filter(|piece| {
                    raw >= PROGRESS_SCALE
                        || !matches!(
                            piece.height_snap,
                            PieceHeightSnap::Search { start: 1, .. }
                                | PieceHeightSnap::Search { end: 1, .. }
                        )
                })
                .map(|piece| &piece.placement)
            {
                let recipient_area = content_area(recipient.frame.inner());
                if donor.content == recipient.content
                    || donor.frame.clip().x != recipient.frame.clip().x
                {
                    continue;
                }
                if raw < PROGRESS_SCALE
                    && endpoint
                        .placements()
                        .find(|piece| piece.content == recipient.content)
                        .is_some_and(|end| {
                            recipient.frame.clip().height.saturating_add(1)
                                > end.frame.clip().height
                        })
                {
                    continue;
                }
                let rows = cells.rows_drawn(&recipient.content, recipient_area);
                let next_rows = cells.rows_drawn(
                    &recipient.content,
                    Rect {
                        height: recipient_area.height.saturating_add(1),
                        ..recipient_area
                    },
                );
                assert!(
                    next_rows <= rows,
                    "{raw}: {:?} keeps {kept}/{} beside {:?} drawing {rows}/{next_rows}; {:?}",
                    donor.content,
                    area.height,
                    recipient.content,
                    column_geometry(drawing)
                );
            }
        }
    }

    /// No incomplete piece leaves an unused row beside a cut column mate.
    fn assert_no_unusable_row(
        cells: &impl TileCells<u32>,
        drawing: &TileDrawing<u32>,
        key: usize,
        raw: u32,
    ) {
        for placement in drawing.placements() {
            let area = content_area(placement.frame.inner());
            let drawn = cells.rows_drawn(&placement.content, area);
            let whole = cells.rows_drawn(
                &placement.content,
                Rect {
                    height: u16::MAX,
                    ..area
                },
            );
            assert!(
                drawn == area.height || drawn >= whole,
                "key {key} at {raw}: {:?} holds span {} but draws {drawn}/{} of {whole}; column {:?}",
                placement.content,
                placement.frame.clip().height.saturating_sub(1),
                area.height,
                column_geometry(drawing),
            );
        }
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
                assert_key_frame_keeps_height_rules(
                    &cells,
                    &before,
                    &after,
                    &drawing,
                    &mut previous,
                );
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

    /// Cell identities and their visible frames in column order.
    fn column_geometry(drawing: &TileDrawing<u32>) -> Vec<(TileContent<u32>, Rect)> {
        drawing
            .placements()
            .filter(|placement| has_visible_body_row(placement.frame))
            .map(|placement| (placement.content.clone(), placement.frame.clip()))
            .collect()
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

    /// Check each sampled frame's body rows and the unchanged cells' paths.
    fn assert_key_frame_keeps_height_rules(
        cells: &impl TileCells<u32>,
        before: &TileDrawing<u32>,
        after: &TileDrawing<u32>,
        drawing: &TileDrawing<u32>,
        previous: &mut [(TileContent<u32>, u16)],
    ) {
        let visible = drawing
            .placements()
            .filter(|placement| has_visible_body_row(placement.frame))
            .map(|placement| placement.frame.clip())
            .collect::<Vec<_>>();
        for pair in visible.windows(2) {
            assert_eq!(
                pair[0].bottom().saturating_sub(1),
                pair[1].top(),
                "neighbours share one edge"
            );
        }
        assert_eq!(
            visible.last().map(|frame| frame.bottom()),
            Some(COLUMN_FOOT_TEST_AREA.bottom())
        );
        for placement in drawing.placements() {
            let height = placement.frame.clip().height;
            assert!(
                height == 1 || height >= MIN_TILE_HEIGHT,
                "a piece takes no edge without a body"
            );
            assert!(
                placement_fits(cells, placement),
                "a piece fills its rows or is whole"
            );
            let start = before
                .placements()
                .find(|held| held.content == placement.content);
            let end = after
                .placements()
                .find(|held| held.content == placement.content);
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let (start, end) = (start.frame.clip(), end.frame.clip());
            assert!(
                (start.height.min(end.height)..=start.height.max(end.height)).contains(&height),
                "{:?} leaves its endpoint heights",
                placement.content
            );
            assert!(placement.frame.clip().x == start.x || placement.frame.clip().x == end.x);
            if let Some((_, preceding)) = previous
                .iter_mut()
                .find(|(content, _)| *content == placement.content)
            {
                assert!(if start.height <= end.height {
                    height >= *preceding
                } else {
                    height <= *preceding
                });
                *preceding = height;
            }
        }
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

    struct SegmentedTitleCells;

    impl TileCells<u32> for SegmentedTitleCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> { title_demands(&[7]) }

        fn group_title(&self, _: &u32) -> Option<Line<'static>> {
            Some(Line::from(vec![
                Span::styled("worker", Style::default().fg(Color::Blue)),
                Span::styled(" · impl", Style::default().fg(Color::Yellow)),
            ]))
        }

        fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
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
            &title_demands(&[7]),
            &[(TileContent::Group(7), 16)],
            TileGridContents::Shown,
            &SegmentedTitleCells,
        );

        assert!(buffer_line(&buffer, 4).contains("worker · impl"));
        assert_eq!(buffer[(1, 4)].fg, Color::Blue);
        assert_eq!(buffer[(8, 4)].fg, Color::Yellow);
        assert_eq!(buffer_text(&buffer).matches("worker · impl").count(), 1);
    }

    #[test]
    fn moving_tiles_keep_title_segments_together_and_inside_their_width() {
        for (width, title) in [(18, "worker · impl"), (14, "worker · im…")] {
            let area = Rect::new(0, 0, width, 8);
            for y in [0_u16, 1, 2] {
                let shift = i32::from(y);
                let drawing = TileDrawing {
                    pieces:       vec![tile_piece(
                        TileContent::Group(7),
                        PaneFrame::shifted(
                            Rect::new(0, 0, width, 4),
                            shift,
                            Rect::new(0, y, width, 4),
                        ),
                        PieceName::Shown,
                    )],
                    column_bands: vec![area],
                };
                let mut buffer = Buffer::empty(area);
                draw_placements(
                    &mut buffer,
                    area,
                    &drawing,
                    &title_demands(&[7]),
                    &[(TileContent::Group(7), width - 2)],
                    TileGridContents::Shown,
                    &SegmentedTitleCells,
                );

                assert!(buffer_line(&buffer, y).contains(title));
                assert_eq!(buffer_text(&buffer).matches(title).count(), 1);
                assert_eq!(buffer[(1, y)].fg, Color::Blue);
                assert_eq!(buffer[(8, y)].fg, Color::Yellow);
                assert!(is_lattice_glyph(buffer[(width - 1, y)].symbol()));
            }
        }
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
    fn a_fading_cells_readout_fades_with_it() {
        let previous_tint = crate::focused_pane_tint_enabled();
        let previous_transparency = crate::transparent_background();
        crate::set_focused_pane_tint(true);
        let area = Rect::new(0, 0, 200, 16);
        for transparent in [false, true] {
            crate::set_transparent_background(transparent);
            let mut cells =
                StubCells::demanding(1, [(7, 100)]).with_summary_foot(SummaryFoot::Text(
                    Line::from(vec![
                        Span::raw(FOOT_TEXT),
                        Span::styled("!", Style::default().fg(success_color())),
                    ])
                    .style(Style::default().fg(label_color())),
                ));
            let (_, unfaded) = settled_grid_in_view(area, &cells, TileView::Cells, MIN_TILE_HEIGHT);
            for amount in [0, 64, 128, u8::MAX] {
                cells.readout_fades = vec![
                    (
                        TileContent::Summary,
                        CellReadoutFade::Fading(CellFadeProgress::new(amount)),
                    ),
                    (
                        TileContent::Group(7),
                        CellReadoutFade::Fading(CellFadeProgress::new(u8::MAX - amount)),
                    ),
                ];
                let (grid, faded) =
                    settled_grid_in_view(area, &cells, TileView::Cells, MIN_TILE_HEIGHT);
                let placements = grid.placements(area, TileGrowth::default());
                assert_eq!(placements.len(), 2);
                for placement in placements {
                    let inner = placement.frame.inner();
                    let foot_y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
                    let progress =
                        CellFadeProgress::new(if placement.content == TileContent::Summary {
                            amount
                        } else {
                            u8::MAX - amount
                        });
                    assert_eq!(
                        cells.readout_fade(&placement.content),
                        CellReadoutFade::Fading(progress)
                    );
                    let ground = pane_background(placement.frame.is_focused());
                    let mut written = 0;
                    for x in inner.left()..inner.right() {
                        let before = &unfaded[(x, foot_y)];
                        let after = &faded[(x, foot_y)];
                        assert_eq!(after.symbol(), before.symbol());
                        if before.symbol() != " " {
                            written += 1;
                            let ink = if before.fg == Color::Reset {
                                text_default()
                            } else {
                                before.fg
                            };
                            assert_eq!(
                                after.fg,
                                blend_color(ink, ground, progress.amount()),
                                "{:?} at {x},{foot_y}; transparent={transparent}",
                                placement.content
                            );
                        }
                    }
                    assert!(written > 0);
                }
            }
        }
        crate::set_focused_pane_tint(previous_tint);
        crate::set_transparent_background(previous_transparency);
    }

    #[test]
    fn summary_foot_is_written_only_on_summary_readout_row() {
        let area = Rect::new(0, 0, 80, 16);
        let cells = StubCells::new([7]).with_summary_foot(SummaryFoot::Text(Line::raw(FOOT_TEXT)));
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
            CellReadoutInk::Unfaded,
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
            CellReadoutInk::Unfaded,
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
                CellReadoutInk::Unfaded,
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
                    CellReadoutInk::Unfaded,
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
            CellReadoutInk::Unfaded,
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
            draw_rows_readout(
                &mut buffer,
                inner,
                22,
                measured_at,
                CellContentExtent::Measured,
                CellReadoutInk::Unfaded,
            );
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
        );
        grid.settle_for_test();
        assert_eq!(grid.placements(tall, growth).len(), 3);
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
        let cells = StubCells::new(1..=5);
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
        let cells = StubCells::new([]).with_summary_foot(SummaryFoot::Text(Line::raw(FOOT_TEXT)));
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

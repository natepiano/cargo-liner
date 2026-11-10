#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;

use ratatui::widgets::Widget;

use super::Buffer;
use super::CellReadoutFade;
use super::CellReadoutInk;
use super::Color;
use super::Display;
use super::Error;
use super::Formatter;
use super::Line;
use super::NonZeroUsize;
use super::Paragraph;
use super::Rect;
use super::Span;
use super::Style;
use super::TILE_FOOT_GAP;
use super::TILE_FOOT_LEFT_INSET;
use super::TILE_ROWS_CELL_LABEL;
use super::TILE_ROWS_CELL_SEPARATOR;
use super::TILE_ROWS_CONTENT_LABEL;
use super::TILE_ROWS_READOUT_HEIGHT;
use super::TILE_ROWS_RIGHT_INSET;
use super::TILE_ROWS_WIDTH_LABEL;
use super::TileCells;
use super::TileContent;
use super::error_color;
use super::label_color;
use super::success_color;
use super::text_default;
/// Content row count, with the current one-based selection when the surface has one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowsReadout {
    /// Measured content rows without a selected row.
    Count(usize),
    /// A selected row checked against the surface's total by [`Self::selected`].
    Selected(SelectedRow),
    /// A checked inclusive range of visible rows.
    Shown(ShownRows),
}

/// The visible rows lie within the measured content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShownRows {
    first: NonZeroUsize,
    last:  NonZeroUsize,
    total: usize,
}

/// A one-based selection that is within its surface's row count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedRow {
    row:  NonZeroUsize,
    rows: usize,
}

impl SelectedRow {
    /// The one-based selected row.
    #[must_use]
    pub const fn row(self) -> NonZeroUsize { self.row }

    /// The number of selectable rows on the surface.
    #[must_use]
    pub const fn rows(self) -> usize { self.rows }
}

/// A selected row falls outside the surface's row count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowExceedsTotal;

impl Display for RowExceedsTotal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("selected row exceeds the surface row count")
    }
}

impl Error for RowExceedsTotal {}

impl RowsReadout {
    /// Check a visible range against the measured content.
    ///
    /// # Errors
    /// Returns [`RowExceedsTotal`] for a reversed range or a row beyond the total.
    pub const fn shown(
        first: NonZeroUsize,
        last: NonZeroUsize,
        total: usize,
    ) -> Result<Self, RowExceedsTotal> {
        if first.get() > last.get() || last.get() > total {
            Err(RowExceedsTotal)
        } else {
            Ok(Self::Shown(ShownRows { first, last, total }))
        }
    }

    /// Check a one-based selection against its surface's total.
    ///
    /// # Errors
    /// Returns [`RowExceedsTotal`] when `row` exceeds `rows`.
    pub const fn selected(row: NonZeroUsize, rows: usize) -> Result<Self, RowExceedsTotal> {
        if row.get() > rows {
            Err(RowExceedsTotal)
        } else {
            Ok(Self::Selected(SelectedRow { row, rows }))
        }
    }
}

/// A cell's foot text and the content measurement used for its fit ink.
#[derive(Clone, Copy)]
pub(super) struct CellRowsReadout {
    pub(super) text:         RowsReadout,
    pub(super) content_rows: usize,
}

impl CellRowsReadout {
    pub(super) const fn new(text: RowsReadout, measured_rows: usize) -> Self {
        Self {
            text,
            // Count providers keep their existing readout_rows adjustment.
            // A selected item's count does not measure its wrapped content.
            content_rows: match text {
                RowsReadout::Count(rows) => rows,
                RowsReadout::Selected(_) | RowsReadout::Shown(_) => measured_rows,
            },
        }
    }

    pub(super) const fn count(rows: usize) -> Self { Self::new(RowsReadout::Count(rows), rows) }
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

/// Source completeness and fading for one cell's readout.
pub(super) struct CellReadout {
    pub(super) extent: CellContentExtent,
    pub(super) ink:    CellReadoutInk,
}

impl CellReadout {
    pub(super) fn for_content<Id>(
        cells: &impl TileCells<Id>,
        content: &TileContent<Id>,
        ground: Color,
    ) -> Self {
        Self {
            extent: cells.content_extent(content),
            ink:    match cells.readout_fade(content) {
                CellReadoutFade::Unfaded => CellReadoutInk::Unfaded,
                CellReadoutFade::Fading(_) => CellReadoutInk::Fading {
                    progress: cells.readout_progress(content),
                    ground,
                },
            },
        }
    }
}

impl From<CellReadoutInk> for CellReadout {
    fn from(ink: CellReadoutInk) -> Self {
        Self {
            extent: CellContentExtent::Measured,
            ink,
        }
    }
}

/// Whether a cell has a distinct row for its foot readout.
pub(super) enum ReadoutRow {
    /// The last interior row, held inside the right inset.
    Reserved(Rect),
    /// The cell has one interior row, or is too narrow for a readout, so
    /// every row belongs to its contents.
    CellTooSmall,
}

/// Write what a cell's contents ask for against what the cell was
/// given, along the separate readout row at the foot of the cell.
///
/// `rows` is the unrounded count the contents would take if nothing
/// stopped them. [`add_readout_rows`](super::measurements::add_readout_rows) adds the readout row
/// before [`TileGrid`](crate::TileGrid) rounds to a step of demand and divides a column by it.
/// `inner.height` is the rows the cell actually has to draw into, one
/// short of its allotment wherever it shares a border with the cell
/// below. The count is written green while the contents fit above the
/// readout and red once they do not.
///
/// The cell's own rows and columns follow, and the width the demand was
/// settled at is written between them only when the two widths differ.
/// The demand is measured against the layout as it stood before
/// [`TileGrid::sync`](crate::TileGrid::sync) and the cell is drawn at whatever the layout
/// became after it -- and a line of text wraps, which makes a demand
/// taken at half the width very nearly twice the rows. So the two
/// widths agreeing is worth no room at all, while the two disagreeing
/// is the whole of what separates a cell asking for too much from a
/// cell asking against the wrong ruler, and is written red where it
/// appears.
pub(super) fn draw_rows_readout(
    buffer: &mut Buffer,
    inner: Rect,
    rows: CellRowsReadout,
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
pub(super) fn draw_rows_readout_after_foot(
    buffer: &mut Buffer,
    inner: Rect,
    rows: CellRowsReadout,
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
pub(super) fn extent_readout_lines(
    inner: Rect,
    rows: CellRowsReadout,
    measured_at: u16,
    extent: CellContentExtent,
) -> Vec<Line<'static>> {
    let reading = if extent == CellContentExtent::Measured
        && rows.content_rows <= usize::from(content_area(inner).height)
    {
        success_color()
    } else {
        error_color()
    };
    let mut content = vec![
        Span::styled(TILE_ROWS_CONTENT_LABEL, Style::default().fg(label_color())),
        Span::styled(
            match rows.text {
                RowsReadout::Count(rows) => rows.to_string(),
                RowsReadout::Selected(selected) => selected.row().to_string(),
                RowsReadout::Shown(shown) => {
                    format!("{}–{} of {}", shown.first, shown.last, shown.total)
                },
            },
            Style::default().fg(reading),
        ),
    ];
    if let RowsReadout::Selected(selected) = rows.text {
        content.extend([
            Span::styled(" of ", Style::default().fg(label_color())),
            Span::styled(selected.rows().to_string(), Style::default().fg(reading)),
        ]);
    }
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
pub(super) fn rows_readout_lines(inner: Rect, rows: usize, measured_at: u16) -> Vec<Line<'static>> {
    extent_readout_lines(
        inner,
        CellRowsReadout::count(rows),
        measured_at,
        CellContentExtent::Measured,
    )
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

/// The cell interior above the readout, or the whole interior when it cannot show one.
pub(in crate::tiles) const fn content_area(inner: Rect) -> Rect {
    match readout_row(inner) {
        ReadoutRow::Reserved(readout) => Rect {
            height: readout.y.saturating_sub(inner.y),
            ..inner
        },
        ReadoutRow::CellTooSmall => inner,
    }
}

/// Rows reserved for the readout at a width that can display it.
pub(super) const fn rows_readout_height(width: u16) -> u16 {
    if width > TILE_ROWS_RIGHT_INSET {
        TILE_ROWS_READOUT_HEIGHT
    } else {
        0
    }
}

/// The last interior row inside the right inset, when a content row remains.
pub(super) const fn readout_row(inner: Rect) -> ReadoutRow {
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

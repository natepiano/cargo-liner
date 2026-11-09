#[cfg(test)]
#[path = "foot_tests.rs"]
mod tests;

use ratatui::widgets::Widget;

use super::Buffer;
use super::CellReadoutInk;
use super::Line;
use super::Paragraph;
use super::ReadoutRow;
use super::Rect;
use super::TILE_FOOT_LEFT_INSET;
use super::readout_row;
/// What the app writes at the left end of the summary cell's readout row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SummaryFoot {
    /// The app writes nothing there.
    Empty,
    /// The app's text, drawn whole from the left inset or not at all.
    Text(Line<'static>),
    /// Draw the prefix whole. Add the suffix only when both fit.
    WithOptionalSuffix {
        /// Text that remains when the suffix does not fit.
        prefix: Line<'static>,
        /// Text drawn only when it fits after the prefix.
        suffix: Line<'static>,
    },
}

/// Whether the summary foot was drawn without clipping.
pub(super) enum SummaryFootDrawOutcome {
    NotDrawn,
    DrawnWhole { width: u16 },
}

/// Draw the app's whole text at the left end of the readout row, or nothing.
pub(super) fn draw_summary_foot(
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

//! Carrying a region of the frame toward the colour it is painted on,
//! and the line the attract screen writes over it when there is no
//! desktop to draw.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

use super::backdrop_notice::BackdropNotice;
use crate::FrameProbe;
use crate::blend_color;
use crate::label_color;
use crate::pane_background;

/// Carry every cell of `area` `faded` of the way toward the colour it
/// is painted on.
///
/// Each cell goes toward its own background rather than toward one
/// colour picked for the grid, so a focused pane's contents settle into
/// the focused pane's tint. [`attract_ground`] stands in only where a
/// cell is painted on nothing at all, which is what a transparent
/// profile leaves behind.
pub fn fade_to_background(buffer: &mut Buffer, area: Rect, faded: u8) {
    let absent = attract_ground();
    for row in area.top()..area.bottom() {
        for column in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((column, row)) {
                let toward = match cell.bg {
                    Color::Reset => absent,
                    background => background,
                };
                cell.set_fg(blend_color(cell.fg, toward, faded));
            }
        }
    }
}

/// The colour anything leaving the attract screen fades toward where
/// the cell it sits on is painted on nothing.
///
/// A profile the app is drawn transparent in paints no ground of its
/// own, and a colour with no channels is one nothing can be mixed
/// against -- so what leaves fades toward black, which is what absent
/// looks like when the desktop is showing through.
#[must_use]
pub fn attract_ground() -> Color {
    match pane_background(false) {
        Color::Reset => Color::Black,
        background => background,
    }
}

/// Say so where the attract screen is running with no desktop to draw.
///
/// Every animation draws in the colours of what is behind the terminal,
/// so with no capture there is nothing to put on the screen and the
/// screen puts nothing there -- which reads as an attract screen that
/// never came on, over a grid that is still sitting where it was. One
/// line is what separates the two. It gives the Screen Recording
/// instruction only when the capture status reports that access was not
/// granted, names stalled-worker recovery directly, and points every
/// other failure to the recorded diagnostics.
///
/// On the last row of `body`, which is the row furthest from anything
/// an idle grid has to say. [`BackdropNotice::None`] draws nothing.
/// [`BackdropNotice::CaptureUnavailable`] is written in `P`'s
/// [`FrameProbe::BACKDROP_UNAVAILABLE_NOTICE`].
pub fn draw_backdrop_notice<P: FrameProbe>(frame: &mut Frame, notice: BackdropNotice, body: Rect) {
    let Some(notice) = notice.text::<P>() else {
        return;
    };
    let Some(row) = body.bottom().checked_sub(1) else {
        return;
    };
    // `set_string` stops at the edge of the buffer, so a terminal too
    // narrow for the whole notice gets as much of it as it can hold
    // rather than a panic or a wrapped second line over the grid.
    frame
        .buffer_mut()
        .set_string(body.left(), row, notice, Style::default().fg(label_color()));
}

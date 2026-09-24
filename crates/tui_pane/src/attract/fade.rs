//! Carrying a region of the frame toward the colour it is painted on.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::blend_color;
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

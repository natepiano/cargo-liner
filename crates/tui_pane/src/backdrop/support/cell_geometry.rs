use crossterm::terminal;

use crate::backdrop::constants::PIXEL_PRECISION;

/// How many pixels one character cell measures across and down, each
/// scaled by [`PIXEL_PRECISION`], or [`None`] where the terminal will
/// not say.
///
/// Wanted by any animation that has to hold a distance steady across
/// the two axes: a cell is taller than it is wide, so the same count of
/// them is a different distance on the screen depending on which way
/// they stack. [`TravelingBand`](crate::backdrop::TravelingBand) reads it to keep its depth when it
/// turns, and [`ResolvingPixels`](crate::backdrop::ResolvingPixels) to make a block read square.
pub(in crate::backdrop) fn cell_pixels() -> Option<(u32, u32)> {
    let size = terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    Some((
        u32::from(size.width) * PIXEL_PRECISION / u32::from(size.columns),
        u32::from(size.height) * PIXEL_PRECISION / u32::from(size.rows),
    ))
}

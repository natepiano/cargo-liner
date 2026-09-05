//! Reducing a display-sized RGBA image to one colour per terminal cell.

use ratatui::style::Color;

use super::CaptureFailure;
use crate::backdrop::constants::SAMPLES_PER_CELL;

/// How many bytes one RGBA pixel occupies.
const BYTES_PER_PIXEL: usize = 4;
/// Where blue sits in an RGBA pixel.
const BLUE: usize = 2;
/// Where green sits in an RGBA pixel.
const GREEN: usize = 1;
/// Where red sits in an RGBA pixel.
const RED: usize = 0;

/// Reduce captured RGBA pixels to the terminal-cell grid implied by `image` and `cell`.
pub(super) fn reduce_capture(
    pixels: &[u8],
    image: (u32, u32),
    cell: (f64, f64),
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let columns =
        whole_cells(f64::from(image.0) / cell.0).ok_or(CaptureFailure::ImageReductionFailed)?;
    let rows =
        whole_cells(f64::from(image.1) / cell.1).ok_or(CaptureFailure::ImageReductionFailed)?;
    let colors =
        reduce(pixels, image, (columns, rows)).ok_or(CaptureFailure::ImageReductionFailed)?;
    Ok((columns, rows, colors))
}

/// How many cells a grid needs to cover a span of that many cells.
///
/// Rounded up so a partial cell at the display edge remains addressable.
fn whole_cells(cells: f64) -> Option<u16> {
    u16::try_from(super::cell_index(cells.ceil())?)
        .ok()
        .filter(|count| *count > 0)
}

/// Average each cell's share of the image down to one colour.
fn reduce(pixels: &[u8], image: (u32, u32), grid: (u16, u16)) -> Option<Vec<Color>> {
    let width = usize::try_from(image.0).ok()?;
    let height = usize::try_from(image.1).ok()?;
    let stride = width.checked_mul(BYTES_PER_PIXEL)?;
    let block = usize::try_from(SAMPLES_PER_CELL).ok()?.max(1);
    let columns = usize::from(grid.0);
    let rows = usize::from(grid.1);
    let mut colors = Vec::with_capacity(columns.checked_mul(rows)?);
    for row in 0..rows {
        let top = row * height / rows;
        let bottom = ((row + 1) * height / rows).max(top + 1);
        for column in 0..columns {
            let left = column * width / columns;
            let right = ((column + 1) * width / columns).max(left + 1);
            let mut totals = [0_u32; 3];
            let mut counted = 0_u32;
            for sample_row in 0..block {
                for sample_column in 0..block {
                    let y = top + (bottom - top) * sample_row / block;
                    let x = left + (right - left) * sample_column / block;
                    let offset = y.checked_mul(stride)?.checked_add(x * BYTES_PER_PIXEL)?;
                    let pixel = pixels.get(offset..offset + BYTES_PER_PIXEL)?;
                    totals[RED] += u32::from(pixel[RED]);
                    totals[GREEN] += u32::from(pixel[GREEN]);
                    totals[BLUE] += u32::from(pixel[BLUE]);
                    counted += 1;
                }
            }
            if counted == 0 {
                return None;
            }
            let channel = |total: u32| u8::try_from(total / counted).unwrap_or(u8::MAX);
            colors.push(Color::Rgb(
                channel(totals[RED]),
                channel(totals[GREEN]),
                channel(totals[BLUE]),
            ));
        }
    }
    Some(colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_reduction_rejects_a_cell_too_large_for_the_image() {
        assert_eq!(
            reduce_capture(&[], (1, 1), (f64::INFINITY, 1.0)),
            Err(CaptureFailure::ImageReductionFailed)
        );
    }

    #[test]
    fn image_reduction_returns_the_implied_grid_and_colors() {
        let pixels = [1, 2, 3, 255, 4, 5, 6, 255];

        assert_eq!(
            reduce_capture(&pixels, (2, 1), (1.0, 1.0)),
            Ok((2, 1, vec![Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)]))
        );
    }
}

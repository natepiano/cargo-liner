/// A distance measured in cells, as a whole number of them.
///
/// The window server measures in floating-point points, and a
/// character cell is a font advance that divides into them evenly
/// only by accident, so a cell index is arrived at as a float and
/// has to be met as one somewhere. This is that place, and the
/// rounding it does is the reason a window dragged by less than a
/// cell does not change what is drawn.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding points to whole cells is what this converts, and \
              a float too large to be a cell index saturates to one no \
              display can hold, which the bounds check rejects"
)]
pub(super) fn cell_index(cells: f64) -> Option<i32> {
    cells.is_finite().then(|| cells.round() as i32)
}

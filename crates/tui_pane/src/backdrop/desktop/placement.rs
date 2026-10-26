/// Where this terminal's character grid sits on the display a
/// [`Desktop`](super::Desktop) covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::backdrop) struct Placement {
    /// The column of the display's grid that the terminal's own column
    /// zero falls in. Signed, because a window dragged past the left
    /// edge puts its first column off the display.
    pub(super) column: i32,
    /// The row of the display's grid that the terminal's own row zero
    /// falls in.
    pub(super) row:    i32,
}

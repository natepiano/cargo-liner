//! Whether the display is live or held still.

/// Whether the display takes new work in as it arrives or is being
/// held still.
///
/// Held, nothing is folded in: no scan, no fade, no cell in motion.
/// Reading a screen that repaints four times a second is what this is
/// for -- a pid pairs off with its parent far more easily when neither
/// of them is about to move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Updates {
    /// Every scan, fade and step is folded in as it reaches the loop.
    Live,
    /// Nothing is folded in until the reader lets go.
    Frozen,
}

impl Updates {
    /// The other of the two, which is all the key that holds the
    /// display asks for.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Live => Self::Frozen,
            Self::Frozen => Self::Live,
        }
    }
}

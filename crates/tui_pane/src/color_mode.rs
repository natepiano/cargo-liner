use crossterm::style::Colored;

/// The terminal colour policy selected at app startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorMode {
    /// Draw with the configured colours.
    Colored,
    /// Draw without terminal colours.
    NoColor,
}

impl ColorMode {
    /// Read the terminal colour policy once at app startup.
    #[must_use]
    pub fn from_environment() -> Self {
        if Colored::ansi_color_disabled_memoized() {
            Self::NoColor
        } else {
            Self::Colored
        }
    }
}

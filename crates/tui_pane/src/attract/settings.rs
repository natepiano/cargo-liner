//! The parameters one attract animation runs with, as a single value.

use super::controller::AttractMode;
use crate::BandSettings;
use crate::PixelSettings;
use crate::TextSettings;

/// Parameters that one attract-screen mode runs with, whether loaded or newly drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttractSettings {
    /// Moving-band parameters.
    MovingBand(BandSettings),
    /// Moving-text parameters.
    MovingText(TextSettings),
    /// Pixelate parameters.
    Pixelate(PixelSettings),
}

impl AttractSettings {
    /// Attract mode that owns these parameters.
    #[must_use]
    pub const fn mode(&self) -> AttractMode {
        match self {
            Self::MovingBand(_) => AttractMode::MovingBand,
            Self::MovingText(_) => AttractMode::MovingText,
            Self::Pixelate(_) => AttractMode::Pixelate,
        }
    }
}

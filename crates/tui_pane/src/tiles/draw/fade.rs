#[cfg(test)]
#[path = "fade_tests.rs"]
mod tests;

use super::BlendRounding;
use super::Color;
use super::Line;
use super::text_default;
use super::theme;
/// How far a cell's ink has travelled toward its ground.
/// Zero retains the ink; [`u8::MAX`] reaches the ground.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellFadeProgress {
    pub(super) amount:   u8,
    pub(super) rounding: BlendRounding,
}

impl CellFadeProgress {
    /// Name an amount on the same scale as [`crate::blend_color`].
    #[must_use]
    pub const fn new(amount: u8) -> Self {
        Self {
            amount,
            rounding: BlendRounding::Truncate,
        }
    }

    /// Entry ink rounds toward its colour on the 256-step brighten scale.
    #[must_use]
    pub const fn brightening(amount: u8) -> Self {
        Self {
            amount,
            rounding: BlendRounding::TowardColor,
        }
    }

    /// Apply this ink state using its own blend scale and channel rounding.
    #[must_use]
    pub fn color(self, color: Color, ground: Color) -> Color {
        theme::blend_color_with_rounding(color, ground, self.amount, self.rounding)
    }

    /// The amount on the zero-to-255 blend scale.
    #[must_use]
    pub const fn amount(self) -> u8 { self.amount }
}

/// Whether a cell's foot follows its contents toward the cell's ground.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CellReadoutFade {
    /// Retain the foot's original inks.
    #[default]
    Unfaded,
    /// Blend the foot's inks toward the cell's ground.
    Fading(CellFadeProgress),
}

/// Ink state shared by every span of a cell's foot.
#[derive(Clone, Copy)]
pub(super) enum CellReadoutInk {
    Unfaded,
    Fading {
        progress: CellFadeProgress,
        ground:   Color,
    },
}

impl CellReadoutInk {
    pub(super) fn line(self, mut line: Line<'static>) -> Line<'static> {
        let Self::Fading { progress, ground } = self else {
            return line;
        };
        for span in &mut line.spans {
            let ink = line.style.patch(span.style).fg.unwrap_or_else(text_default);
            span.style = span.style.fg(progress.color(ink, ground));
        }
        line
    }
}

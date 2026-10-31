//! Clamped progress percentages.

/// A bar fraction clamped to `0..=100`, computed once from a subset count
/// so the renderer never re-divides and can never exceed the bar width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percentage(u8);

impl Percentage {
    /// An empty (0%) percentage.
    #[must_use]
    pub const fn empty() -> Self { Self(0) }

    /// A complete (100%) percentage.
    #[must_use]
    pub const fn full() -> Self { Self(100) }

    /// Percentage of `expected` keys that have been seen. An empty
    /// `expected` set renders 100; `seen >= expected` clamps to 100.
    #[must_use]
    pub fn from_fraction(seen: usize, expected: usize) -> Self {
        if expected == 0 || seen >= expected {
            return Self(100);
        }
        // seen < expected and expected > 0, so the quotient is in 0..100.
        Self(u8::try_from(seen * 100 / expected).unwrap_or(100))
    }

    /// The clamped value from zero to one hundred.
    #[must_use]
    pub const fn get(self) -> u8 { self.0 }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn percentage_from_fraction_clamps_and_handles_empty() {
        assert_eq!(
            Percentage::from_fraction(0, 0).get(),
            100,
            "empty expected renders 100%"
        );
        assert_eq!(
            Percentage::from_fraction(5, 3).get(),
            100,
            "overshoot clamps to 100%"
        );
        assert_eq!(Percentage::from_fraction(0, 4).get(), 0);
        assert_eq!(Percentage::from_fraction(1, 4).get(), 25);
        assert_eq!(
            Percentage::from_fraction(3, 8).get(),
            37,
            "fractional percent floors"
        );
    }
}

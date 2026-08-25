//! The cheap varying number the attract-mode animations are drawn
//! from, and the characters they draw.
//!
//! Shared rather than owned by either animation: a strip crossing the
//! grid and a window filled with drifting lines want the same texture,
//! and two generators seeded a microsecond apart would give it to them
//! twice.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::constants::GLYPHS;
use super::constants::XORSHIFT_FALLBACK_SEED;
use super::constants::XORSHIFT_FIRST;
use super::constants::XORSHIFT_SECOND;
use super::constants::XORSHIFT_THIRD;

/// Xorshift64, seeded from the clock.
///
/// The character churn needs a cheap varying number and nothing more,
/// so this stands in for a dependency on a real generator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Xorshift(u64);

impl Default for Xorshift {
    fn default() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| {
                u64::try_from(since_epoch.as_nanos()).unwrap_or(0)
            });
        Self(if seed == 0 {
            XORSHIFT_FALLBACK_SEED
        } else {
            seed
        })
    }
}

impl Xorshift {
    /// The same generator started from a seed of the caller's choosing,
    /// so a test can read the field a given draw produces rather than
    /// whatever the clock happened to hand over.
    #[cfg(test)]
    pub(super) const fn seeded(seed: u64) -> Self {
        Self(if seed == 0 {
            XORSHIFT_FALLBACK_SEED
        } else {
            seed
        })
    }

    /// The next number in the sequence.
    pub(super) const fn roll(&mut self) -> u64 {
        self.0 ^= self.0 << XORSHIFT_FIRST;
        self.0 ^= self.0 >> XORSHIFT_SECOND;
        self.0 ^= self.0 << XORSHIFT_THIRD;
        self.0
    }

    /// A number across the whole of [`u8`], which is the scale both
    /// animations hold a per-line or per-offset draw on.
    pub(super) fn byte(&mut self) -> u8 {
        u8::try_from(self.index(usize::from(u8::MAX) + 1)).unwrap_or(u8::MAX)
    }

    /// A number in `0..len`, or zero where `len` is zero.
    pub(super) fn index(&mut self, len: usize) -> usize {
        let Ok(len) = u64::try_from(len) else {
            return 0;
        };
        if len == 0 {
            return 0;
        }
        usize::try_from(self.roll() % len).unwrap_or(0)
    }
}

/// One character drawn at random from [`GLYPHS`].
pub(super) fn random_glyph(xorshift: &mut Xorshift) -> char {
    let index = xorshift.index(GLYPHS.len());
    GLYPHS.get(index).copied().unwrap_or(' ')
}

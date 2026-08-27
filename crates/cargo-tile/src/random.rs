//! Dependency-free randomness for app-owned choices.

use std::num::NonZeroUsize;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
const SPLITMIX_FIRST_MULTIPLIER: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX_SECOND_MULTIPLIER: u64 = 0x94d0_49bb_1331_11eb;

/// A nonempty set of consecutive indices beginning at zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroIndexBound(NonZeroUsize);

impl NonZeroIndexBound {
    /// Build an index bound from a collection length.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyIndexDomain`] when `len` contains no index to draw.
    pub(crate) const fn try_from_len(len: usize) -> Result<Self, EmptyIndexDomain> {
        match NonZeroUsize::new(len) {
            Some(bound) => Ok(Self(bound)),
            None => Err(EmptyIndexDomain),
        }
    }

    const fn get(self) -> usize { self.0.get() }
}

/// A requested index draw had no possible result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EmptyIndexDomain;

/// Seed a caller-owned random operation from nanoseconds since the Unix epoch.
#[must_use]
pub(crate) fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since_epoch| {
            u64::try_from(since_epoch.as_nanos()).unwrap_or(u64::MAX)
        })
}

/// Draw an unbiased index in `0..bound` from a reproducible seed.
#[must_use]
pub(crate) fn bounded_index(seed: u64, bound: NonZeroIndexBound) -> usize {
    let mut generator = SplitMix64::new(seed);
    bounded_index_from(&mut generator, bound)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self { Self(seed) }

    const fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(SPLITMIX_INCREMENT);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(SPLITMIX_FIRST_MULTIPLIER);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(SPLITMIX_SECOND_MULTIPLIER);
        mixed ^ (mixed >> 31)
    }
}

fn bounded_index_from(generator: &mut SplitMix64, bound: NonZeroIndexBound) -> usize {
    let bound = u64::try_from(bound.get()).unwrap_or(u64::MAX);
    let rejected_tail_len = (u64::MAX % bound).saturating_add(1) % bound;
    let last_accepted = u64::MAX - rejected_tail_len;
    loop {
        let candidate = generator.draw();
        if candidate <= last_accepted {
            return usize::try_from(candidate % bound).unwrap_or(0);
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn empty_lengths_are_rejected_before_drawing() {
        assert_eq!(NonZeroIndexBound::try_from_len(0), Err(EmptyIndexDomain));
        assert_eq!(
            NonZeroIndexBound::try_from_len(1),
            Ok(NonZeroIndexBound(NonZeroUsize::MIN))
        );
    }

    #[test]
    fn a_fixed_seed_corpus_reaches_every_bounded_index_deterministically() {
        let bound = NonZeroIndexBound::try_from_len(7).expect("seven is nonzero");
        let corpus = 0_u64..=255;
        let indices = corpus
            .clone()
            .map(|seed| bounded_index(seed, bound))
            .collect::<HashSet<_>>();

        assert_eq!(indices, (0..7).collect());
        for seed in corpus {
            assert_eq!(bounded_index(seed, bound), bounded_index(seed, bound));
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn a_seed_in_the_short_tail_is_rejected_and_redrawn() {
        let bound_len = usize::MAX / 2 + 2;
        let bound = NonZeroIndexBound::try_from_len(bound_len).expect("bound is nonzero");
        let mut generator = SplitMix64::new(0);
        let first = generator.draw();
        let second = generator.draw();
        let bound_u64 = u64::try_from(bound_len).expect("64-bit usize fits u64");
        let rejected_tail_len = (u64::MAX % bound_u64).saturating_add(1) % bound_u64;
        let last_accepted = u64::MAX - rejected_tail_len;

        assert!(first > last_accepted, "the first draw must enter the tail");
        assert!(
            second <= last_accepted,
            "the second draw must leave the tail"
        );
        assert_eq!(
            bounded_index(0, bound),
            usize::try_from(second % bound_u64).expect("reduced draw fits usize")
        );
        assert_ne!(
            bounded_index(0, bound),
            usize::try_from(first).expect("64-bit draw fits usize") % bound_len
        );
    }
}

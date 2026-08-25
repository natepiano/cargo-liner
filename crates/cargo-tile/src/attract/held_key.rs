//! How a key held down arrives, and what one press of it is worth.
//!
//! Shared by every [`AttractMode`](super::AttractMode): each animation
//! has its own actions and its own run in progress, so the run is
//! generic over what it is a run of.

use std::time::Instant;

use crate::constants::HELD_KEY_GAP;
use crate::constants::HELD_KEY_MAX_STEP;
use crate::constants::HELD_KEY_PRESSES_PER_STEP;

/// How far into a run of presses of the same key the reader is.
///
/// A terminal reports a held key as a run of presses arriving a few
/// tens of milliseconds apart rather than as a press and a release, so
/// there is nothing to ask how long a key has been down for -- the run
/// itself is the measurement. Presses arriving inside [`HELD_KEY_GAP`]
/// of each other continue the run; anything slower, or a different
/// action, starts a fresh one.
///
/// What the run buys is size: a key held down moves the band further
/// per press the longer it is held, so crossing the whole range of
/// widths or speeds does not cost sixty presses.
#[derive(Debug)]
pub(crate) struct HeldKey<A> {
    /// The action the run is made of, or [`None`] before the first
    /// press of the session.
    action:     Option<A>,
    /// When the last press of the run arrived.
    pressed_at: Instant,
    /// How many presses into the run the last one was.
    presses:    u32,
}

impl<A: Copy + PartialEq> HeldKey<A> {
    /// A run that has not started.
    pub(crate) fn new() -> Self {
        Self {
            action:     None,
            pressed_at: Instant::now(),
            presses:    0,
        }
    }

    /// Fold a press of `action` arriving at `pressed_at` into the run,
    /// and say how many steps it is worth.
    ///
    /// Never fewer than one, so a single press always does something,
    /// and never more than [`HELD_KEY_MAX_STEP`], so a key left down
    /// settles into a steady climb rather than running away from the
    /// reader.
    pub(crate) fn step(&mut self, action: A, pressed_at: Instant) -> u32 {
        let continuing = self.action == Some(action)
            && pressed_at.duration_since(self.pressed_at) <= HELD_KEY_GAP;
        self.presses = if continuing {
            self.presses.saturating_add(1)
        } else {
            1
        };
        self.action = Some(action);
        self.pressed_at = pressed_at;
        (self.presses / HELD_KEY_PRESSES_PER_STEP).clamp(1, HELD_KEY_MAX_STEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attract::moving_band::MovingBandAction;

    /// A gap short enough to read as the same key still being held.
    const HELD: std::time::Duration = HELD_KEY_GAP;

    #[test]
    fn a_single_press_is_worth_one_step() {
        let mut held_key = HeldKey::new();

        assert_eq!(held_key.step(MovingBandAction::Wider, Instant::now()), 1);
    }

    /// A run of presses is worth more per press the longer it runs, so
    /// a key held down crosses the range without sixty presses.
    #[test]
    fn a_run_of_presses_grows_the_step_and_then_stops_growing() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        let mut steps = Vec::new();

        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * (HELD_KEY_MAX_STEP + 2)) {
            steps.push(held_key.step(MovingBandAction::Wider, pressed_at));
            pressed_at += HELD;
        }

        assert_eq!(steps.first(), Some(&1));
        assert_eq!(steps.last(), Some(&HELD_KEY_MAX_STEP));
        assert!(
            steps.windows(2).all(|pair| pair[0] <= pair[1]),
            "the step should only ever grow within one run: {steps:?}",
        );
    }

    /// Turning to a different key starts the run over. Otherwise a long
    /// hold on `+` would leave the next press of `-` taking eight steps
    /// back the other way.
    #[test]
    fn a_different_key_starts_the_run_over() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * HELD_KEY_MAX_STEP) {
            held_key.step(MovingBandAction::Wider, pressed_at);
            pressed_at += HELD;
        }

        assert_eq!(held_key.step(MovingBandAction::Thinner, pressed_at), 1);
    }

    /// A press that arrives after the reader has let go is the start of
    /// a new run, not the continuation of the old one.
    #[test]
    fn a_press_after_the_gap_starts_the_run_over() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * HELD_KEY_MAX_STEP) {
            held_key.step(MovingBandAction::Wider, pressed_at);
            pressed_at += HELD;
        }

        let let_go = pressed_at + HELD_KEY_GAP * 2;

        assert_eq!(held_key.step(MovingBandAction::Wider, let_go), 1);
    }
}

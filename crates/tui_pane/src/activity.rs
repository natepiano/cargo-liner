//! Framework-owned activity indicators.

use std::time::Duration;

pub use crate::constants::ACTIVITY_SPINNER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cycle {
    period: Duration,
}

impl Cycle {
    const fn new(period: Duration) -> Self {
        assert!(
            period.as_secs() > 0 || period.subsec_nanos() > 0,
            "animation cycle period must be non-zero"
        );
        Self { period }
    }
}

/// A fixed set of frames sampled over a fixed period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCycle {
    frames: &'static [&'static str],
    cycle:  Cycle,
}

impl FrameCycle {
    /// Construct a frame cycle from static frames and a non-zero period.
    ///
    /// # Panics
    ///
    /// Panics when `frames` is empty or `period` is zero.
    #[must_use]
    pub const fn new(frames: &'static [&'static str], period: Duration) -> Self {
        assert!(
            !frames.is_empty(),
            "frame cycle requires at least one frame"
        );
        Self {
            frames,
            cycle: Cycle::new(period),
        }
    }

    /// Return the frame for `elapsed`, wrapping at the cycle period.
    #[must_use]
    pub fn frame_at(self, elapsed: Duration) -> &'static str {
        let frame_count = u128::try_from(self.frames.len()).unwrap_or(u128::MAX);
        let period = self.cycle.period.as_nanos();
        let elapsed = elapsed.as_nanos() % period;
        let frame_index = elapsed.saturating_mul(frame_count) / period;
        let frame_index = usize::try_from(frame_index).unwrap_or(self.frames.len() - 1);
        self.frames[frame_index]
    }

    /// Return the elapsed-time boundary where the frame next changes.
    pub(crate) fn next_frame_boundary(self, elapsed: Duration) -> Duration {
        let frame_count = u128::try_from(self.frames.len()).unwrap_or(u128::MAX);
        let period = self.cycle.period.as_nanos();
        let elapsed_nanos = elapsed.as_nanos();
        let completed_cycles = elapsed_nanos / period;
        let elapsed_in_cycle = elapsed_nanos % period;
        let frame_index = elapsed_in_cycle.saturating_mul(frame_count) / period;
        let next_in_cycle = frame_index
            .saturating_add(1)
            .saturating_mul(period)
            .div_ceil(frame_count);
        duration_from_nanos(
            completed_cycles
                .saturating_mul(period)
                .saturating_add(next_in_cycle),
        )
    }
}

/// A static or animated icon suitable for pane rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// A fixed icon.
    Static(&'static str),
    /// An icon whose frame is selected from elapsed time.
    Animated(FrameCycle),
}

impl Icon {
    /// Return the current icon frame.
    #[must_use]
    pub fn frame_at(self, elapsed: Duration) -> &'static str {
        match self {
            Self::Static(icon) => icon,
            Self::Animated(cycle) => cycle.frame_at(elapsed),
        }
    }
}

fn duration_from_nanos(nanos: u128) -> Duration {
    let nanos_per_second = Duration::from_secs(1).as_nanos();
    let seconds = nanos / nanos_per_second;
    let subsecond_nanos = nanos % nanos_per_second;
    let Ok(seconds) = u64::try_from(seconds) else {
        return Duration::MAX;
    };
    Duration::new(seconds, u32::try_from(subsecond_nanos).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FrameCycle;

    const TEST_FRAMES: &[&str] = &["a", "b", "c", "d"];
    const TEST_FRAME_CYCLE: FrameCycle = FrameCycle::new(TEST_FRAMES, Duration::from_millis(400));

    #[test]
    fn frame_cycle_returns_first_frame_at_zero() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::ZERO), "a");
    }

    #[test]
    fn frame_cycle_advances_after_each_interval() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(100)), "b");
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(200)), "c");
    }

    #[test]
    fn frame_cycle_wraps_after_full_period() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(400)), "a");
    }

    #[test]
    fn frame_cycle_reports_the_next_frame_boundary() {
        assert_eq!(
            TEST_FRAME_CYCLE.next_frame_boundary(Duration::ZERO),
            Duration::from_millis(100)
        );
        assert_eq!(
            TEST_FRAME_CYCLE.next_frame_boundary(Duration::from_millis(100)),
            Duration::from_millis(200)
        );
        assert_eq!(
            TEST_FRAME_CYCLE.next_frame_boundary(Duration::from_millis(399)),
            Duration::from_millis(400)
        );
    }
}

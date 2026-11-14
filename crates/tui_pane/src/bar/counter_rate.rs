//! Counter rates between framework status ticks.

use std::time::Duration;
use std::time::Instant;

/// A cumulative counter that can measure its positive change.
pub(super) trait CumulativeCounter: Copy {
    fn increase_since(self, previous: Self) -> f64;
}

impl CumulativeCounter for Duration {
    fn increase_since(self, previous: Self) -> f64 { self.saturating_sub(previous).as_secs_f64() }
}

impl CumulativeCounter for u64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "A byte rate needs a floating-point approximation of the full counter increase."
    )]
    fn increase_since(self, previous: Self) -> f64 { self.saturating_sub(previous) as f64 }
}

/// Whether the current process counter is available.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CounterReading<C> {
    Unavailable,
    Available(C),
}

/// Whether two counter observations provide a rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CounterRate {
    Pending,
    Measured(f64),
}

/// The previous successful cumulative counter observation.
enum CounterSample<C> {
    Pending,
    Recorded { at: Instant, total: C },
}

/// One rate calculation shared by CPU time and disk bytes.
pub(super) struct CounterRateSampler<C> {
    reading:  CounterRate,
    previous: CounterSample<C>,
}

impl<C: CumulativeCounter> CounterRateSampler<C> {
    pub(super) const fn new() -> Self {
        Self {
            reading:  CounterRate::Pending,
            previous: CounterSample::Pending,
        }
    }

    pub(super) const fn reading(&self) -> CounterRate { self.reading }

    pub(super) fn sample(&mut self, now: Instant, counter: CounterReading<C>) {
        let CounterReading::Available(total) = counter else {
            self.previous = CounterSample::Pending;
            self.reading = CounterRate::Pending;
            return;
        };
        if let CounterSample::Recorded {
            at,
            total: previous,
        } = self.previous
        {
            let elapsed = now.saturating_duration_since(at);
            if elapsed.is_zero() {
                return;
            }
            self.reading =
                CounterRate::Measured(total.increase_since(previous) / elapsed.as_secs_f64());
        }
        self.previous = CounterSample::Recorded { at: now, total };
    }
}

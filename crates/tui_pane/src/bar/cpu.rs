//! The current process CPU reading for the framework status line.

use std::time::Duration;
use std::time::Instant;

use rustix::time::ClockId;

use super::counter_rate::CounterRate;
use super::counter_rate::CounterRateSampler;
use super::counter_rate::CounterReading;
use super::rate_label;

/// Whether two CPU samples provide a reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CpuReading {
    Pending,
    Measured(f64),
}

impl CpuReading {
    pub(super) fn label(self) -> String {
        match self {
            Self::Pending => "—".to_string(),
            Self::Measured(percent) => format!("{}%", rate_label::rate_label(percent)),
        }
    }
}

/// CPU time used by this process, including all its threads.
pub(crate) type ProcessCpuTime = CounterReading<Duration>;

impl ProcessCpuTime {
    /// Read user and system CPU time through the safe process clock.
    pub(crate) fn read() -> Self {
        Duration::try_from(rustix::time::clock_gettime(ClockId::ProcessCPUTime))
            .map_or(Self::Unavailable, Self::Available)
    }
}

/// Samples the current process across framework status ticks.
pub(crate) struct CpuSampler {
    rate: CounterRateSampler<Duration>,
}

impl CpuSampler {
    pub(crate) const fn new() -> Self {
        Self {
            rate: CounterRateSampler::new(),
        }
    }

    pub(crate) const fn reading(&self) -> CpuReading {
        match self.rate.reading() {
            CounterRate::Pending => CpuReading::Pending,
            CounterRate::Measured(rate) => CpuReading::Measured(rate * 100.0),
        }
    }

    pub(crate) fn sample(&mut self, now: Instant, cpu: ProcessCpuTime) {
        self.rate.sample(now, cpu);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::CpuReading;
    use super::CpuSampler;
    use super::ProcessCpuTime;

    #[test]
    fn the_process_cpu_reading_uses_cpu_time_over_wall_time() {
        let start = Instant::now();
        let mut sampler = CpuSampler::new();
        assert_eq!(sampler.reading(), CpuReading::Pending);
        sampler.sample(start, ProcessCpuTime::Available(Duration::from_secs(4)));
        assert_eq!(sampler.reading(), CpuReading::Pending);
        for (second, milliseconds, expected) in
            [(1, 4100, 10.0), (2, 5100, 100.0), (4, 9100, 200.0)]
        {
            sampler.sample(
                start + Duration::from_secs(second),
                ProcessCpuTime::Available(Duration::from_millis(milliseconds)),
            );
            assert_eq!(sampler.reading(), CpuReading::Measured(expected));
        }
    }

    #[test]
    fn the_process_cpu_label_formats_pending_and_both_precisions() {
        for (reading, expected) in [
            (CpuReading::Pending, "—"),
            (CpuReading::Measured(9.9), "9.9%"),
            (CpuReading::Measured(9.94), "9.9%"),
            (CpuReading::Measured(9.96), "10%"),
            (CpuReading::Measured(10.0), "10%"),
            (CpuReading::Measured(100.0), "100%"),
        ] {
            assert_eq!(reading.label(), expected);
        }
    }
}

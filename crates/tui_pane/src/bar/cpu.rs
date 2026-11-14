//! The current process CPU reading for the framework status line.

use std::time::Duration;
use std::time::Instant;

#[cfg(unix)]
use rustix::time::ClockId;

use super::constants::CPU_DECIMAL_BELOW_PERCENT;

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
            // Round to tenths first, so 9.96 shows as `10%`, not `10.0%`.
            Self::Measured(percent)
                if (percent * 10.0).round() / 10.0 < CPU_DECIMAL_BELOW_PERCENT =>
            {
                format!("{percent:.1}%")
            },
            Self::Measured(percent) => format!("{percent:.0}%"),
        }
    }
}

/// CPU time used by this process, including all its threads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ProcessCpuTime {
    Unavailable,
    Available(Duration),
}

impl ProcessCpuTime {
    /// Read user and system CPU time through the safe process clock.
    #[cfg(unix)]
    pub(crate) fn read() -> Self {
        Duration::try_from(rustix::time::clock_gettime(ClockId::ProcessCPUTime))
            .map_or(Self::Unavailable, Self::Available)
    }

    /// Windows has no process clock in rustix, so the status line shows no reading.
    #[cfg(not(unix))]
    pub(crate) const fn read() -> Self { Self::Unavailable }
}

/// The previous successful process CPU sample.
enum CpuSample {
    Pending,
    Recorded {
        at:       Instant,
        cpu_time: Duration,
    },
}

/// Samples the current process across framework status ticks.
pub(crate) struct CpuSampler {
    reading:  CpuReading,
    previous: CpuSample,
}

impl CpuSampler {
    pub(crate) const fn new() -> Self {
        Self {
            reading:  CpuReading::Pending,
            previous: CpuSample::Pending,
        }
    }

    pub(crate) const fn reading(&self) -> CpuReading { self.reading }

    pub(crate) fn sample(&mut self, now: Instant, cpu: ProcessCpuTime) {
        let ProcessCpuTime::Available(cpu_time) = cpu else {
            self.previous = CpuSample::Pending;
            self.reading = CpuReading::Pending;
            return;
        };
        if let CpuSample::Recorded {
            at,
            cpu_time: previous_cpu,
        } = self.previous
        {
            let elapsed = now.saturating_duration_since(at);
            if elapsed.is_zero() {
                return;
            }
            self.reading = CpuReading::Measured(
                cpu_time.saturating_sub(previous_cpu).as_secs_f64() / elapsed.as_secs_f64() * 100.0,
            );
        }
        self.previous = CpuSample::Recorded { at: now, cpu_time };
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
            (CpuReading::Measured(10.0), "10%"),
            (CpuReading::Measured(100.0), "100%"),
        ] {
            assert_eq!(reading.label(), expected);
        }
    }
}

//! The current process storage read rate for the status line.

use std::time::Instant;

use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;

use super::constants::DISK_READ_UNIT_BASE;
use super::counter_rate::CounterRate;
use super::counter_rate::CounterRateSampler;
use super::counter_rate::CounterReading;
use super::rate_label;

/// Whether two storage read counters provide a byte rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DiskReadRate {
    Pending,
    Measured(f64),
}

impl DiskReadRate {
    pub(super) fn label(self) -> String {
        let Self::Measured(mut bytes) = self else {
            return "—".to_string();
        };
        let mut unit = "B";
        for next in ["KB", "MB", "GB", "TB", "PB", "EB"] {
            if bytes < DISK_READ_UNIT_BASE {
                break;
            }
            bytes /= DISK_READ_UNIT_BASE;
            unit = next;
        }
        if unit == "B" {
            format!("{bytes:.0} {unit}")
        } else {
            format!("{} {unit}", rate_label::rate_label(bytes))
        }
    }
}

/// Holds one sysinfo process table across status ticks.
pub(crate) struct DiskReadSampler {
    system: System,
    pid:    Pid,
    rate:   CounterRateSampler<u64>,
}

impl DiskReadSampler {
    pub(crate) fn new() -> Self {
        Self {
            system: System::new(),
            pid:    Pid::from_u32(std::process::id()),
            rate:   CounterRateSampler::new(),
        }
    }

    pub(crate) const fn reading(&self) -> DiskReadRate {
        match self.rate.reading() {
            CounterRate::Pending => DiskReadRate::Pending,
            CounterRate::Measured(rate) => DiskReadRate::Measured(rate),
        }
    }

    pub(crate) fn sample(&mut self, now: Instant) {
        let refreshed = self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing()
                .with_disk_usage()
                .without_tasks(),
        );
        let counter = if refreshed == 0 {
            CounterReading::Unavailable
        } else {
            self.system
                .process(self.pid)
                .map_or(CounterReading::Unavailable, |process| {
                    CounterReading::Available(process.disk_usage().total_read_bytes)
                })
        };
        self.rate.sample(now, counter);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::DiskReadRate;
    use super::DiskReadSampler;
    use crate::bar::counter_rate::CounterReading;

    #[test]
    fn the_process_disk_read_rate_uses_bytes_over_wall_time() {
        let start = Instant::now();
        let mut sampler = DiskReadSampler::new();
        assert_eq!(sampler.reading(), DiskReadRate::Pending);
        for (second, counter, expected) in [
            (0, CounterReading::Available(1000), DiskReadRate::Pending),
            (
                1,
                CounterReading::Available(1000),
                DiskReadRate::Measured(0.0),
            ),
            (
                3,
                CounterReading::Available(2024),
                DiskReadRate::Measured(512.0),
            ),
            (
                4,
                CounterReading::Available(14_024),
                DiskReadRate::Measured(12_000.0),
            ),
            (
                6,
                CounterReading::Available(6_814_024),
                DiskReadRate::Measured(3_400_000.0),
            ),
            (
                7,
                CounterReading::Available(76_814_024),
                DiskReadRate::Measured(70_000_000.0),
            ),
            (8, CounterReading::Unavailable, DiskReadRate::Pending),
            (
                9,
                CounterReading::Available(76_814_024),
                DiskReadRate::Pending,
            ),
            (
                10,
                CounterReading::Available(76_814_024),
                DiskReadRate::Measured(0.0),
            ),
        ] {
            sampler
                .rate
                .sample(start + Duration::from_secs(second), counter);
            assert_eq!(sampler.reading(), expected, "second {second}");
        }
    }

    #[test]
    fn the_process_disk_read_label_formats_pending_and_decimal_units() {
        for (reading, expected) in [
            (DiskReadRate::Pending, "—"),
            (DiskReadRate::Measured(0.0), "0 B"),
            (DiskReadRate::Measured(512.0), "512 B"),
            (DiskReadRate::Measured(1200.0), "1.2 KB"),
            (DiskReadRate::Measured(9940.0), "9.9 KB"),
            (DiskReadRate::Measured(9960.0), "10 KB"),
            (DiskReadRate::Measured(12_000.0), "12 KB"),
            (DiskReadRate::Measured(3_400_000.0), "3.4 MB"),
            (DiskReadRate::Measured(70_000_000.0), "70 MB"),
        ] {
            assert_eq!(reading.label(), expected);
        }
    }
}

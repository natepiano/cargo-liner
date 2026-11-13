//! `DiskReadMeasurement` measures reads on the current task when trace logging is active.

use std::fmt::Display;
use std::fmt::Formatter;

use tui_pane::PERF_LOG_TARGET;

#[derive(Clone, Copy, Debug)]
pub(crate) enum DiskReadBytes {
    Measured(u64),
    Unavailable,
}

pub(crate) enum DiskReadMeasurement {
    Active(u64),
    Disabled,
}

impl DiskReadMeasurement {
    pub(crate) fn enabled() -> bool {
        tracing::enabled!(target: PERF_LOG_TARGET, tracing::Level::TRACE)
    }

    pub(crate) fn start() -> Self {
        if Self::enabled() {
            Self::read_bytes().map_or(Self::Disabled, Self::Active)
        } else {
            Self::Disabled
        }
    }

    pub(crate) fn finish(self) -> DiskReadBytes {
        match self {
            Self::Active(before) => Self::read_bytes()
                .map_or(DiskReadBytes::Unavailable, |after| {
                    DiskReadBytes::Measured(after.saturating_sub(before))
                }),
            Self::Disabled => DiskReadBytes::Unavailable,
        }
    }

    #[cfg(target_os = "linux")]
    fn read_bytes() -> Option<u64> {
        std::fs::read_to_string("/proc/thread-self/io")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("read_bytes:")?.trim().parse().ok())
    }

    #[cfg(not(target_os = "linux"))]
    const fn read_bytes() -> Option<u64> { None }
}

impl DiskReadBytes {
    pub(crate) const fn plus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Measured(left), Self::Measured(right)) => {
                Self::Measured(left.saturating_add(right))
            },
            _ => Self::Unavailable,
        }
    }
}

impl Display for DiskReadBytes {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Measured(bytes) => bytes.fmt(formatter),
            Self::Unavailable => formatter.write_str("unavailable"),
        }
    }
}

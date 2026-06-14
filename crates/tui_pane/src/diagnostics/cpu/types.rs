/// Per-core CPU usage sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuCoreUsage {
    /// Display label for the core (typically "CPU N").
    pub label:   String,
    /// Utilization percentage rounded to a `u8` in `0..=100`.
    pub percent: u8,
}

/// Aggregate CPU/GPU sample produced by `CpuPoller::poll`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuUsage {
    /// Aggregate CPU utilization across all cores, in `0..=100`.
    pub total_percent: u8,
    /// Per-core breakdown.
    pub cores:         Vec<CpuCoreUsage>,
    /// System/user/idle percentage breakdown computed from raw ticks.
    pub breakdown:     CpuBreakdown,
    /// Latest GPU utilization, when available on this OS.
    pub gpu_percent:   Option<u8>,
}

impl CpuUsage {
    /// Build a zero-filled snapshot with `core_count` placeholder cores.
    #[must_use]
    pub fn placeholder(core_count: usize) -> Self {
        Self {
            total_percent: 0,
            cores:         (0..core_count)
                .map(|index| CpuCoreUsage {
                    label:   format!("CPU {}", index + 1),
                    percent: 0,
                })
                .collect(),
            breakdown:     CpuBreakdown::default(),
            gpu_percent:   None,
        }
    }
}

/// System / user / idle CPU-time percentage breakdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuBreakdown {
    /// Percentage of CPU time spent in kernel mode.
    pub system: u8,
    /// Percentage of CPU time spent in user mode.
    pub user:   u8,
    /// Percentage of CPU time spent idle.
    pub idle:   u8,
}

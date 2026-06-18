use super::CpuBreakdownRaw;
use super::CpuRefreshKind;
#[cfg(target_os = "linux")]
use super::FdinfoGpuSampler;
#[cfg(target_os = "windows")]
use super::GpuQuery;
use super::RefreshKind;
use super::RollingMean;
use super::System;
use super::cpu_breakdown;
use super::cpu_percent;
use super::normalize_cpu_label;
use super::read_cpu_breakdown_raw;
#[cfg(not(target_os = "windows"))]
use super::read_gpu_usage;

/// Sysinfo-backed CPU/GPU sampler.
///
/// Each [`poll`](Self::poll) refreshes the sysinfo [`System`], computes
/// the system/user/idle breakdown from raw ticks, and samples GPU
/// utilization. The sampler does not gate its own cadence — that is
/// owned by `CpuMonitor`, which drives a poller on a worker thread.
#[derive(Debug)]
pub struct CpuPoller {
    system:                 System,
    last_breakdown_raw:     CpuBreakdownRaw,
    /// Rolling window over GPU device samples; an unavailable poll leaves it
    /// untouched rather than diluting the mean.
    gpu_device_smoothing:   RollingMean,
    /// Rolling window over GPU renderer samples, when the platform exposes it.
    gpu_renderer_smoothing: RollingMean,
    /// Rolling window over GPU tiler samples, when the platform exposes it.
    gpu_tiler_smoothing:    RollingMean,
    /// Persistent PDH query for GPU utilization (Windows only).
    #[cfg(target_os = "windows")]
    gpu_query:              Option<GpuQuery>,
    /// DRM `fdinfo` engine-utilization sampler (Linux fallback).
    #[cfg(target_os = "linux")]
    fdinfo_gpu:             FdinfoGpuSampler,
}

impl Default for CpuPoller {
    fn default() -> Self { Self::new() }
}

impl CpuPoller {
    /// Construct a poller, priming the sysinfo and breakdown baselines
    /// so the first [`poll`](Self::poll) reports a real delta.
    #[must_use]
    pub fn new() -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );
        system.refresh_cpu_all();
        Self {
            system,
            last_breakdown_raw: read_cpu_breakdown_raw(),
            gpu_device_smoothing: RollingMean::default(),
            gpu_renderer_smoothing: RollingMean::default(),
            gpu_tiler_smoothing: RollingMean::default(),
            #[cfg(target_os = "windows")]
            gpu_query: GpuQuery::new(),
            #[cfg(target_os = "linux")]
            fdinfo_gpu: FdinfoGpuSampler::new(),
        }
    }

    /// Number of CPU cores reported by the underlying [`System`], floored at 1.
    #[must_use]
    pub fn core_count(&self) -> usize { self.system.cpus().len().max(1) }

    /// Sample CPU and GPU utilization now, relative to the previous poll.
    pub fn poll(&mut self) -> CpuUsage {
        self.system.refresh_cpu_all();

        let cores = self
            .system
            .cpus()
            .iter()
            .enumerate()
            .map(|(index, cpu)| CpuCoreUsage {
                label:   normalize_cpu_label(cpu.name(), index),
                percent: cpu_percent(cpu.cpu_usage()),
            })
            .collect::<Vec<_>>();

        let total_percent = cpu_percent(self.system.global_cpu_usage());
        let breakdown = cpu_breakdown(&mut self.last_breakdown_raw);
        #[cfg(target_os = "windows")]
        let gpu = GpuUsage::from_device_percent(self.gpu_query.as_ref().and_then(GpuQuery::sample));
        #[cfg(target_os = "linux")]
        let gpu = {
            let mut usage = read_gpu_usage();
            if usage.device_percent.is_none() {
                usage.device_percent = self.fdinfo_gpu.sample();
            }
            usage
        };
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let gpu = read_gpu_usage();
        let gpu = smooth_gpu_usage(
            gpu,
            &mut self.gpu_device_smoothing,
            &mut self.gpu_renderer_smoothing,
            &mut self.gpu_tiler_smoothing,
        );

        CpuUsage {
            total_percent,
            cores,
            breakdown,
            gpu,
        }
    }
}

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
    /// Latest GPU metrics, when available on this OS.
    pub gpu:           GpuUsage,
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
            gpu:           GpuUsage::default(),
        }
    }
}

/// GPU metrics sampled by the platform-specific backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUsage {
    /// Number of physical GPU cores, when the OS exposes it.
    pub core_count:       Option<u16>,
    /// Aggregate device utilization percentage.
    pub device_percent:   Option<u8>,
    /// Renderer utilization percentage, currently exposed on macOS.
    pub renderer_percent: Option<u8>,
    /// Tiler utilization percentage, currently exposed on macOS.
    pub tiler_percent:    Option<u8>,
}

impl GpuUsage {
    #[must_use]
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    pub(super) const fn from_device_percent(device_percent: Option<u8>) -> Self {
        Self {
            core_count: None,
            device_percent,
            renderer_percent: None,
            tiler_percent: None,
        }
    }
}

fn smooth_gpu_usage(
    usage: GpuUsage,
    device_smoothing: &mut RollingMean,
    renderer_smoothing: &mut RollingMean,
    tiler_smoothing: &mut RollingMean,
) -> GpuUsage {
    let smooth = |percent: Option<u8>, smoothing: &mut RollingMean| {
        percent.map(|percent| cpu_percent(smoothing.push(f32::from(percent))))
    };
    GpuUsage {
        core_count:       usage.core_count,
        device_percent:   smooth(usage.device_percent, device_smoothing),
        renderer_percent: smooth(usage.renderer_percent, renderer_smoothing),
        tiler_percent:    smooth(usage.tiler_percent, tiler_smoothing),
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

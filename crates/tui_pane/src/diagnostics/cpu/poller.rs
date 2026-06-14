use super::CpuBreakdownRaw;
use super::CpuCoreUsage;
use super::CpuRefreshKind;
use super::CpuUsage;
use super::RefreshKind;
use super::RollingMean;
use super::System;
use super::cpu_breakdown;
use super::cpu_percent;
use super::normalize_cpu_label;
use super::read_cpu_breakdown_raw;
use super::read_gpu_percent;

/// Sysinfo-backed CPU/GPU sampler.
///
/// Each [`poll`](Self::poll) refreshes the sysinfo [`System`], computes
/// the system/user/idle breakdown from raw ticks, and samples GPU
/// utilization. The sampler does not gate its own cadence — that is
/// owned by `CpuMonitor`, which drives a poller on a worker thread.
#[derive(Debug)]
pub struct CpuPoller {
    system:             System,
    last_breakdown_raw: CpuBreakdownRaw,
    /// Rolling window over GPU samples; an unavailable poll leaves it
    /// untouched rather than diluting the mean.
    gpu_smoothing:      RollingMean,
    /// Persistent PDH query for GPU utilization (Windows only).
    #[cfg(target_os = "windows")]
    gpu_query:          Option<GpuQuery>,
    /// DRM `fdinfo` engine-utilization sampler (Linux fallback).
    #[cfg(target_os = "linux")]
    fdinfo_gpu:         FdinfoGpuSampler,
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
            gpu_smoothing: RollingMean::default(),
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
        let gpu_percent = self.gpu_query.as_ref().and_then(GpuQuery::sample);
        #[cfg(target_os = "linux")]
        let gpu_percent = read_gpu_percent().or_else(|| self.fdinfo_gpu.sample());
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let gpu_percent = read_gpu_percent();
        let gpu_percent =
            gpu_percent.map(|percent| cpu_percent(self.gpu_smoothing.push(f32::from(percent))));

        CpuUsage {
            total_percent,
            cores,
            breakdown,
            gpu_percent,
        }
    }
}

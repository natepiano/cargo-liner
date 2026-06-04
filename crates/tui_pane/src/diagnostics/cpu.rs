//! Sysinfo-backed CPU/GPU sampler plus severity buckets keyed to the
//! framework's theme colors.
//!
//! [`CpuMonitor`] owns a background worker thread that runs a
//! [`CpuPoller`] on a fixed cadence and forwards each sampled
//! [`CpuUsage`] over a channel. The render thread only performs a
//! non-blocking [`latest`](CpuMonitor::latest) drain, so frame paints
//! never block on the sysinfo / `GetSystemTimes` / PDH syscalls the
//! poll performs. [`severity`] maps a percentage to a [`CpuSeverity`]
//! bucket using caller-supplied thresholds; [`CpuSeverity::color`]
//! resolves to the framework's success / title / error theme colors.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::collections::HashSet;
use std::collections::VecDeque;
#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::ptr::from_ref;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::RecvTimeoutError;
use crossbeam_channel::Sender;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFDictionary;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFNumber;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFRetained;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFString;
#[cfg(target_os = "macos")]
use objc2_core_foundation::CFType;
#[cfg(target_os = "macos")]
use objc2_core_foundation::kCFAllocatorDefault;
#[cfg(target_os = "macos")]
use objc2_io_kit::IOIteratorNext;
#[cfg(target_os = "macos")]
use objc2_io_kit::IOObjectRelease;
#[cfg(target_os = "macos")]
use objc2_io_kit::IORegistryEntryCreateCFProperty;
#[cfg(target_os = "macos")]
use objc2_io_kit::IOServiceGetMatchingServices;
#[cfg(target_os = "macos")]
use objc2_io_kit::IOServiceMatching;
#[cfg(target_os = "macos")]
use objc2_io_kit::io_iterator_t;
#[cfg(target_os = "macos")]
use objc2_io_kit::kIOMainPortDefault;
use ratatui::style::Color;
use sysinfo::CpuRefreshKind;
use sysinfo::RefreshKind;
use sysinfo::System;

use crate::theme;

/// Per-core CPU usage sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuCoreUsage {
    /// Display label for the core (typically "CPU N").
    pub label:   String,
    /// Utilization percentage rounded to a `u8` in `0..=100`.
    pub percent: u8,
}

/// Aggregate CPU/GPU sample produced by [`CpuPoller::poll`].
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

/// Severity bucket for a CPU utilization percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuSeverity {
    /// Below the green-max threshold.
    Green,
    /// Between green-max and yellow-max.
    Yellow,
    /// Above the yellow-max threshold.
    Red,
}

impl CpuSeverity {
    /// Resolve this severity to its framework theme color.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Green => theme::success_color(),
            Self::Yellow => theme::title_color(),
            Self::Red => theme::error_color(),
        }
    }
}

/// How many poll samples a [`RollingMean`] window averages.
///
/// A workload running in ~1 s bursts aliases against a 1 s poll cadence
/// — the instantaneous sample swings wildly with phase alignment. At the
/// 1 s cadence this window reads as the average over the last 5 s.
pub const CPU_SMOOTHING_WINDOW_POLLS: usize = 5;

/// Bounded rolling-mean window for utilization samples.
///
/// Damps single-poll spikes and the transient zeros the macOS GPU
/// counter publishes mid-update. Used for the GPU row here and for the
/// per-process CPU column in consumers' process lists.
#[derive(Clone, Debug, Default)]
pub struct RollingMean {
    window: VecDeque<f32>,
}

impl RollingMean {
    /// Fold `sample` into the window and return the new mean. The first
    /// sample is the mean of one — no zero dilution.
    pub fn push(&mut self, sample: f32) -> f32 {
        self.window.push_back(sample);
        if self.window.len() > CPU_SMOOTHING_WINDOW_POLLS {
            self.window.pop_front();
        }
        let len = u16::try_from(self.window.len()).unwrap_or(u16::MAX);
        self.window.iter().sum::<f32>() / f32::from(len)
    }
}

/// Sysinfo-backed CPU/GPU sampler.
///
/// Each [`poll`](Self::poll) refreshes the sysinfo [`System`], computes
/// the system/user/idle breakdown from raw ticks, and samples GPU
/// utilization. The sampler does not gate its own cadence — that is
/// owned by [`CpuMonitor`], which drives a poller on a worker thread.
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

/// Background CPU/GPU sampler.
///
/// Spawns a worker thread that runs a [`CpuPoller`] on a fixed cadence
/// and forwards each [`CpuUsage`] over a channel. The render thread
/// calls [`latest`](Self::latest) — a non-blocking drain — so it never
/// blocks on the syscalls a poll performs. Dropping the monitor stops
/// the worker and joins it.
#[derive(Debug)]
pub struct CpuMonitor {
    samples:     Receiver<CpuUsage>,
    stop_sender: Sender<()>,
    handle:      Option<JoinHandle<()>>,
    core_count:  usize,
}

impl CpuMonitor {
    /// Spawn the worker thread, sampling at most every
    /// `poll_interval_ms` milliseconds (floored at 1ms).
    ///
    /// The poller is primed on the calling thread so [`core_count`]
    /// is available immediately; the first sample arrives one interval
    /// later, by which point the worker has a real delta window.
    ///
    /// [`core_count`]: Self::core_count
    #[must_use]
    pub fn new(poll_interval_ms: u64) -> Self {
        let mut poller = CpuPoller::new();
        let core_count = poller.core_count();
        let interval = Duration::from_millis(poll_interval_ms.max(1));
        let (sample_sender, samples) = crossbeam_channel::unbounded();
        let (stop_sender, stop_receiver) = crossbeam_channel::unbounded();
        let handle = thread::Builder::new()
            .name("cpu-monitor".to_string())
            .spawn(move || cpu_poll_loop(&mut poller, &sample_sender, &stop_receiver, interval))
            .ok();
        Self {
            samples,
            stop_sender,
            handle,
            core_count,
        }
    }

    /// Number of CPU cores, captured when the worker was spawned.
    #[must_use]
    pub const fn core_count(&self) -> usize { self.core_count }

    /// Whether the worker thread spawned successfully and is producing
    /// samples. When `false` (the `thread::Builder::spawn` failed and
    /// the sample `Sender` was dropped with the unrun closure), the
    /// [`receiver`](Self::receiver) is permanently disconnected — the
    /// event loop must not register it in a `Select`, or the loop would
    /// busy-spin on a perpetually-ready dead channel.
    #[must_use]
    pub const fn is_sampling(&self) -> bool { self.handle.is_some() }

    /// The sample channel receiver, for registering in a render-loop
    /// `crossbeam_channel::Select` so a new sample wakes the loop.
    ///
    /// Register only — draining is exclusive to [`latest`](Self::latest),
    /// which the render thread calls each frame. Registering does not
    /// consume; the `Select` merely signals readiness. Gate registration
    /// on [`is_sampling`](Self::is_sampling).
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<CpuUsage> { &self.samples }

    /// Zero-filled [`CpuUsage`] sized to the current core count.
    #[must_use]
    pub fn placeholder_cpu_usage(&self) -> CpuUsage { CpuUsage::placeholder(self.core_count) }

    /// Drain the channel, returning the most recent sample if any
    /// arrived since the last call. Never blocks.
    #[must_use]
    pub fn latest(&self) -> Option<CpuUsage> {
        let mut newest = None;
        while let Ok(usage) = self.samples.try_recv() {
            newest = Some(usage);
        }
        newest
    }
}

impl Drop for CpuMonitor {
    fn drop(&mut self) {
        // Wake the worker out of its timed wait so the join is prompt;
        // a closed channel (worker already exited) is fine to ignore.
        let _ = self.stop_sender.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Worker loop: wait up to `interval` for a stop signal; on timeout,
/// sample and forward. Exits when stop is signaled, when the monitor
/// is dropped (stop channel disconnects), or when the sample channel
/// is closed.
fn cpu_poll_loop(
    poller: &mut CpuPoller,
    samples: &Sender<CpuUsage>,
    stop: &Receiver<()>,
    interval: Duration,
) {
    // `recv_timeout` returns `Timeout` each interval (sample and forward);
    // `Ok`/`Disconnected` means stop was signaled or the monitor dropped.
    while stop.recv_timeout(interval) == Err(RecvTimeoutError::Timeout) {
        if samples.send(poller.poll()).is_err() {
            break;
        }
    }
}

/// Number of filled 10%-bucket cells for a given percentage,
/// rounding up.
#[must_use]
pub fn filled_cells(percent: u8) -> usize {
    let clamped = if percent > 100 { 100 } else { percent };
    usize::from(clamped).div_ceil(10)
}

/// Map a percentage to a [`CpuSeverity`] using caller-supplied thresholds.
#[must_use]
pub const fn severity(percent: u8, green_max_percent: u8, yellow_max_percent: u8) -> CpuSeverity {
    if percent <= green_max_percent {
        CpuSeverity::Green
    } else if percent <= yellow_max_percent {
        CpuSeverity::Yellow
    } else {
        CpuSeverity::Red
    }
}

/// Color used to render the empty (unfilled) cells of a CPU bar.
#[must_use]
pub fn blank_bar_color() -> Color { theme::inactive_border_color() }

fn cpu_percent(value: f32) -> u8 { rounded_percent(f64::from(value)) }

fn normalize_cpu_label(name: &str, index: usize) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        format!("CPU {}", index + 1)
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CpuBreakdownRaw {
    system: u64,
    user:   u64,
    idle:   u64,
}

fn cpu_breakdown(previous: &mut CpuBreakdownRaw) -> CpuBreakdown {
    let current = read_cpu_breakdown_raw();
    let delta_system = current.system.saturating_sub(previous.system);
    let delta_user = current.user.saturating_sub(previous.user);
    let delta_idle = current.idle.saturating_sub(previous.idle);
    let delta_total = delta_system
        .saturating_add(delta_user)
        .saturating_add(delta_idle);
    *previous = current;

    if delta_total == 0 {
        return CpuBreakdown::default();
    }

    CpuBreakdown {
        system: percent_from_parts(delta_system, delta_total),
        user:   percent_from_parts(delta_user, delta_total),
        idle:   percent_from_parts(delta_idle, delta_total),
    }
}

fn percent_from_parts(value: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let rounded = value.saturating_mul(100).saturating_add(total / 2) / total;
    bounded_percent_u8(rounded)
}

fn rounded_percent(value: f64) -> u8 {
    let clamped = value.clamp(0.0, 100.0);
    let mut percent = 0u8;
    while percent < 100 && f64::from(percent) + 0.5 <= clamped {
        percent += 1;
    }
    percent
}

fn bounded_percent_u8(value: u64) -> u8 { u8::try_from(value.min(100)).unwrap_or(100) }

#[cfg(target_os = "macos")]
fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { macos_cpu_breakdown_raw() }

#[cfg(target_os = "linux")]
fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { linux_cpu_breakdown_raw() }

#[cfg(target_os = "windows")]
fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { windows_cpu_breakdown_raw() }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { CpuBreakdownRaw::default() }

/// `kern_return_t` success code shared by the mach and `IOKit` calls below.
#[cfg(target_os = "macos")]
const KERN_SUCCESS: i32 = 0;

/// Query the I/O Registry directly for the first `IOAccelerator` service
/// whose `PerformanceStatistics` dictionary reports a device utilization
/// percentage. Replaces spawning `ioreg` on every poll.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "IOKit FFI replaces the per-poll ioreg spawn")]
fn read_gpu_percent() -> Option<u8> {
    // SAFETY: the argument is a valid NUL-terminated C string the call
    // copies into the returned matching dictionary.
    let matching = unsafe { IOServiceMatching(c"IOAccelerator".as_ptr()) }?;
    // `IOServiceGetMatchingServices` consumes one dictionary reference, so
    // hand it a second retain and let `matching` release the original.
    let matching = CFRetained::<CFDictionary>::from(&*matching);

    let mut services: io_iterator_t = 0;
    // SAFETY: `services` is a valid out-param; the call consumes the
    // `matching` reference handed to it.
    let result = unsafe {
        IOServiceGetMatchingServices(kIOMainPortDefault, Some(matching), &raw mut services)
    };
    if result != KERN_SUCCESS {
        return None;
    }

    let mut gpu_percent = None;
    loop {
        let accelerator = IOIteratorNext(services);
        if accelerator == 0 {
            break;
        }
        // SAFETY: `accelerator` is a live service handle from
        // `IOIteratorNext`, released right below; the key outlives the call.
        let statistics = unsafe {
            IORegistryEntryCreateCFProperty(
                accelerator,
                Some(&CFString::from_static_str("PerformanceStatistics")),
                kCFAllocatorDefault,
                0,
            )
        };
        IOObjectRelease(accelerator);
        if let Some(statistics) = statistics
            && let Ok(statistics) = statistics.downcast::<CFDictionary>()
            && let Some(percent) = device_utilization_percent(&statistics)
        {
            gpu_percent = Some(percent);
            break;
        }
    }
    IOObjectRelease(services);
    gpu_percent
}

#[cfg(not(target_os = "macos"))]
#[cfg(target_os = "linux")]
fn read_gpu_percent() -> Option<u8> { linux_sysfs_gpu_percent().or_else(linux_nvidia_gpu_percent) }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_gpu_percent() -> Option<u8> { None }

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "CoreFoundation dictionary FFI for the GPU statistics lookup"
)]
fn device_utilization_percent(statistics: &CFDictionary) -> Option<u8> {
    let key = CFString::from_static_str("Device Utilization %");
    // SAFETY: the key pointer must target the `CFString` object itself, not
    // the `CFRetained` wrapper — hence the explicit deref target; the
    // returned pointer is borrowed from `statistics`, which outlives it.
    let value = unsafe { statistics.value(from_ref::<CFString>(&key).cast()) };
    if value.is_null() {
        return None;
    }
    // SAFETY: a non-null value out of a CF dictionary is a valid CF object;
    // `downcast_ref` verifies the concrete type before any use.
    let number = unsafe { &*value.cast::<CFType>() }.downcast_ref::<CFNumber>()?;
    let percent = u64::try_from(number.as_i64()?).ok()?;
    Some(bounded_percent_u8(percent))
}

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "mach host_statistics FFI for the system/user/idle tick split"
)]
fn macos_cpu_breakdown_raw() -> CpuBreakdownRaw {
    type Integer = i32;
    type Natural = u32;
    type MachPort = u32;
    type MachMsgCount = u32;
    type Host = MachPort;
    type HostFlavor = i32;
    type KernReturn = i32;

    const CPU_STATE_USER: usize = 0;
    const CPU_STATE_SYSTEM: usize = 1;
    const CPU_STATE_IDLE: usize = 2;
    const CPU_STATE_MAX: usize = 4;
    const HOST_CPU_LOAD_INFO: HostFlavor = 3;

    #[repr(C)]
    struct HostCpuLoadInfo {
        cpu_ticks: [Natural; CPU_STATE_MAX],
    }

    unsafe extern "C" {
        fn mach_host_self() -> Host;
        fn host_statistics(
            host_priv: Host,
            flavor: HostFlavor,
            host_info_out: *mut Integer,
            host_info_out_count: *mut MachMsgCount,
        ) -> KernReturn;
    }

    let mut info = HostCpuLoadInfo {
        cpu_ticks: [0; CPU_STATE_MAX],
    };
    let Some(mut count) = MachMsgCount::try_from(
        std::mem::size_of::<HostCpuLoadInfo>() / std::mem::size_of::<Integer>(),
    )
    .ok() else {
        return CpuBreakdownRaw::default();
    };

    // SAFETY: `info` is a writable buffer of exactly the size `count`
    // reports in `Integer` units, per the `host_statistics` contract.
    let result = unsafe {
        host_statistics(
            mach_host_self(),
            HOST_CPU_LOAD_INFO,
            (&raw mut info).cast::<Integer>(),
            &raw mut count,
        )
    };

    if result != KERN_SUCCESS {
        return CpuBreakdownRaw::default();
    }

    CpuBreakdownRaw {
        system: u64::from(info.cpu_ticks[CPU_STATE_SYSTEM]),
        user:   u64::from(info.cpu_ticks[CPU_STATE_USER]),
        idle:   u64::from(info.cpu_ticks[CPU_STATE_IDLE]),
    }
}

#[cfg(target_os = "linux")]
fn linux_cpu_breakdown_raw() -> CpuBreakdownRaw {
    let contents = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let Some(line) = contents.lines().find(|line| line.starts_with("cpu ")) else {
        return CpuBreakdownRaw::default();
    };
    parse_linux_proc_stat_line(line).unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_stat_line(line: &str) -> Option<CpuBreakdownRaw> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let user = parts.next()?.parse::<u64>().ok()?;
    let nice = parts.next()?.parse::<u64>().ok()?;
    let system = parts.next()?.parse::<u64>().ok()?;
    let idle = parts.next()?.parse::<u64>().ok()?;
    Some(CpuBreakdownRaw {
        system,
        user: user.saturating_add(nice),
        idle,
    })
}

#[cfg(target_os = "linux")]
fn linux_sysfs_gpu_percent() -> Option<u8> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path().join("device").join("gpu_busy_percent");
            let value = std::fs::read_to_string(path).ok()?;
            value.trim().parse::<u8>().ok()
        })
        .max()
        .map(|value| value.min(100))
}

#[cfg(target_os = "linux")]
fn linux_nvidia_gpu_percent() -> Option<u8> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u8>().ok())
        .max()
        .map(|value| value.min(100))
}

/// Cumulative GPU busy nanoseconds parsed from one DRM `fdinfo` file.
#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq, Eq)]
struct DrmClientSample {
    /// `drm-client-id`, used to dedupe a client that holds several fds.
    client_id:      Option<i64>,
    /// `(engine name, cumulative busy ns)` per `drm-engine-<name>` line.
    engine_busy_ns: Vec<(String, u64)>,
}

/// Parse the `drm-` usage-stats keys from one `/proc/<pid>/fdinfo/<fd>`
/// file. Returns `None` for any fd that exposes no `drm-engine-*` lines:
/// non-DRM fds, and DRM drivers that emit no engine utilization (the
/// Apple `asahi` driver among them).
#[cfg(target_os = "linux")]
fn parse_drm_fdinfo(contents: &str) -> Option<DrmClientSample> {
    let mut client_id = None;
    let mut engine_busy_ns = Vec::new();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "drm-client-id" {
            client_id = value.parse::<i64>().ok();
        } else if let Some(engine) = key.strip_prefix("drm-engine-") {
            // `drm-engine-capacity-<name>` is an instance count, not busy time.
            if engine.starts_with("capacity-") {
                continue;
            }
            // The value reads "<nanoseconds> ns"; take the leading integer.
            if let Some(busy_ns) = value
                .split_whitespace()
                .next()
                .and_then(|token| token.parse::<u64>().ok())
            {
                engine_busy_ns.push((engine.to_string(), busy_ns));
            }
        }
    }
    if engine_busy_ns.is_empty() {
        return None;
    }
    Some(DrmClientSample {
        client_id,
        engine_busy_ns,
    })
}

/// Sum cumulative busy nanoseconds per engine across every readable DRM
/// client in `/proc`. A client that holds multiple fds repeats identical
/// totals, so dedupe by `drm-client-id` before summing. The map is empty
/// when no DRM client exposes engine stats.
#[cfg(target_os = "linux")]
fn collect_drm_engine_busy_ns() -> HashMap<String, u64> {
    let mut totals: HashMap<String, u64> = HashMap::new();
    let mut seen_clients: HashSet<i64> = HashSet::new();
    let Ok(process_dirs) = std::fs::read_dir("/proc") else {
        return totals;
    };
    for process_dir in process_dirs.filter_map(Result::ok) {
        let Ok(fdinfo_entries) = std::fs::read_dir(process_dir.path().join("fdinfo")) else {
            continue;
        };
        for fdinfo_entry in fdinfo_entries.filter_map(Result::ok) {
            let Ok(contents) = std::fs::read_to_string(fdinfo_entry.path()) else {
                continue;
            };
            let Some(sample) = parse_drm_fdinfo(&contents) else {
                continue;
            };
            if let Some(client_id) = sample.client_id
                && !seen_clients.insert(client_id)
            {
                continue;
            }
            for (engine, busy_ns) in sample.engine_busy_ns {
                let total = totals.entry(engine).or_insert(0);
                *total = total.saturating_add(busy_ns);
            }
        }
    }
    totals
}

/// Stateful GPU sampler over DRM `fdinfo` engine utilization.
///
/// The kernel reports cumulative per-engine busy nanoseconds; utilization
/// is the busiest engine's delta over the wall-clock interval between
/// polls. This is the driver-agnostic fallback used when neither
/// `gpu_busy_percent` (AMD) nor `nvidia-smi` is available. It reports
/// nothing when no DRM client exposes engine stats — including the Apple
/// `asahi` driver, which implements no `fdinfo` utilization.
#[cfg(target_os = "linux")]
#[derive(Debug)]
struct FdinfoGpuSampler {
    previous_busy_ns:    HashMap<String, u64>,
    previous_sampled_at: Instant,
}

#[cfg(target_os = "linux")]
impl FdinfoGpuSampler {
    /// Prime the baseline from the current engine totals.
    fn new() -> Self {
        Self {
            previous_busy_ns:    collect_drm_engine_busy_ns(),
            previous_sampled_at: Instant::now(),
        }
    }

    /// Sample the busiest engine's utilization since the previous poll,
    /// in `0..=100`. Returns `None` when no DRM client exposes engine
    /// stats (this driver provides no `fdinfo` utilization).
    fn sample(&mut self) -> Option<u8> {
        let current = collect_drm_engine_busy_ns();
        let now = Instant::now();
        let elapsed_ns = now
            .saturating_duration_since(self.previous_sampled_at)
            .as_nanos();
        let busiest_delta_ns = current
            .iter()
            .map(|(engine, busy_ns)| {
                busy_ns.saturating_sub(self.previous_busy_ns.get(engine).copied().unwrap_or(0))
            })
            .max();
        self.previous_busy_ns = current;
        self.previous_sampled_at = now;
        engine_busy_percent(busiest_delta_ns?, elapsed_ns)
    }
}

/// Busy nanoseconds over an elapsed-nanoseconds window, as a percentage
/// in `0..=100`. `None` when the window is zero.
#[cfg(target_os = "linux")]
fn engine_busy_percent(busy_ns: u64, elapsed_ns: u128) -> Option<u8> {
    if elapsed_ns == 0 {
        return None;
    }
    let percent = u128::from(busy_ns).saturating_mul(100) / elapsed_ns;
    Some(bounded_percent_u8(u64::try_from(percent).unwrap_or(100)))
}

#[cfg(target_os = "windows")]
#[allow(
    unsafe_code,
    reason = "Win32 GetSystemTimes FFI for the system/user/idle tick split"
)]
fn windows_cpu_breakdown_raw() -> CpuBreakdownRaw {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        dw_low_date_time:  u32,
        dw_high_date_time: u32,
    }

    type Bool = i32;

    unsafe extern "system" {
        fn GetSystemTimes(
            idle_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> Bool;
    }

    fn file_time_to_u64(value: FileTime) -> u64 {
        (u64::from(value.dw_high_date_time) << 32) | u64::from(value.dw_low_date_time)
    }

    let mut idle_time = FileTime {
        dw_low_date_time:  0,
        dw_high_date_time: 0,
    };
    let mut kernel_time = FileTime {
        dw_low_date_time:  0,
        dw_high_date_time: 0,
    };
    let mut user_time = FileTime {
        dw_low_date_time:  0,
        dw_high_date_time: 0,
    };

    // SAFETY: each argument is a valid, writable `FileTime` local; GetSystemTimes
    // only writes through the pointers and reports success via a nonzero return.
    let ok =
        unsafe { GetSystemTimes(&raw mut idle_time, &raw mut kernel_time, &raw mut user_time) };
    if ok == 0 {
        return CpuBreakdownRaw::default();
    }

    let idle = file_time_to_u64(idle_time);
    let kernel = file_time_to_u64(kernel_time);
    let user = file_time_to_u64(user_time);

    CpuBreakdownRaw {
        system: kernel.saturating_sub(idle),
        user,
        idle,
    }
}

/// Wildcard PDH counter path summing 3-D engine utilization across
/// every GPU engine instance. Uses the English (non-localized) counter
/// names so the query resolves regardless of the system language.
#[cfg(target_os = "windows")]
const GPU_COUNTER_PATH: &str = "\\GPU Engine(*engtype_3D)\\Utilization Percentage";

/// `ERROR_SUCCESS` / `PDH_CSTATUS_VALID_DATA`.
#[cfg(target_os = "windows")]
const PDH_SUCCESS: u32 = 0x0000_0000;
/// A per-item `CStatus` indicating a freshly cooked sample.
#[cfg(target_os = "windows")]
const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;
/// `PdhGetFormattedCounterArrayW` needs a larger buffer (sizing pass).
#[cfg(target_os = "windows")]
const PDH_MORE_DATA: u32 = 0x8000_07D2;
/// Request cooked counter values formatted as `f64`.
#[cfg(target_os = "windows")]
const PDH_FMT_DOUBLE: u32 = 0x0000_0200;

/// Rust mirror of `PDH_FMT_COUNTERVALUE` (the `double` union arm). The
/// 8-byte alignment of `f64` reproduces the 4-byte pad C inserts after
/// `CStatus`, so `double_value` lands at the union's offset.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Debug)]
struct PdhFmtCounterValue {
    c_status:     u32,
    double_value: f64,
}

/// Rust mirror of `PDH_FMT_COUNTERVALUE_ITEM_W`.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Debug)]
struct PdhFmtCounterValueItem {
    name:  *mut u16,
    value: PdhFmtCounterValue,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code, reason = "PDH FFI for the GPU engine counters")]
#[link(name = "pdh")]
unsafe extern "system" {
    fn PdhOpenQueryW(data_source: *const u16, user_data: usize, query: *mut isize) -> u32;
    fn PdhAddEnglishCounterW(
        query: isize,
        counter_path: *const u16,
        user_data: usize,
        counter: *mut isize,
    ) -> u32;
    fn PdhCollectQueryData(query: isize) -> u32;
    fn PdhGetFormattedCounterArrayW(
        counter: isize,
        format: u32,
        buffer_size: *mut u32,
        item_count: *mut u32,
        item_buffer: *mut PdhFmtCounterValueItem,
    ) -> u32;
    fn PdhCloseQuery(query: isize) -> u32;
}

/// A persistent PDH query for GPU 3-D engine utilization.
///
/// Replaces the previous per-poll `powershell Get-Counter` spawn that
/// cost ~2.5s on the render thread. The query stays open across polls,
/// so each [`sample`](Self::sample) collects a second data point
/// relative to the previous poll — no process spawn, no sleep, and the
/// utilization is cooked over the natural poll interval.
#[cfg(target_os = "windows")]
#[derive(Debug)]
struct GpuQuery {
    query:   isize,
    counter: isize,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code, reason = "PDH FFI for the GPU engine counters")]
impl GpuQuery {
    /// Open the query, add the wildcard counter, and prime the baseline
    /// sample. Returns `None` if PDH or the GPU counter is unavailable.
    fn new() -> Option<Self> {
        let path: Vec<u16> = GPU_COUNTER_PATH
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut query: isize = 0;
        // SAFETY: `query` is a valid out-param; a null data source selects the
        // live system, per the PdhOpenQueryW contract.
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &raw mut query) } != PDH_SUCCESS {
            return None;
        }
        let mut counter: isize = 0;
        // SAFETY: `query` is the handle just opened; `path` is a NUL-terminated
        // UTF-16 string that outlives the call; `counter` is a valid out-param.
        if unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &raw mut counter) }
            != PDH_SUCCESS
        {
            // SAFETY: `query` is the live handle returned by PdhOpenQueryW.
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        // Rate counters need two samples to cook a value; prime the baseline.
        // SAFETY: `query` is a live, open query handle.
        unsafe { PdhCollectQueryData(query) };
        Some(Self { query, counter })
    }

    /// Collect a fresh sample and sum the cooked utilization across all
    /// matching engine instances, clamped to `0..=100`.
    fn sample(&self) -> Option<u8> {
        // SAFETY: `self.query` stays valid for the lifetime of `self`.
        if unsafe { PdhCollectQueryData(self.query) } != PDH_SUCCESS {
            return None;
        }

        // Sizing pass: a NULL buffer yields the bytes and item count needed.
        let mut buffer_size: u32 = 0;
        let mut item_count: u32 = 0;
        // SAFETY: `self.counter` is live; the size/count out-params are valid;
        // a null item buffer requests only the required sizes (PDH_MORE_DATA).
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &raw mut buffer_size,
                &raw mut item_count,
                std::ptr::null_mut(),
            )
        };
        if status != PDH_MORE_DATA || buffer_size == 0 {
            return None;
        }

        // Allocate as `PdhFmtCounterValueItem` so the buffer is correctly
        // aligned; PDH appends the instance-name strings in the tail bytes.
        let elem = std::mem::size_of::<PdhFmtCounterValueItem>();
        let capacity = (buffer_size as usize).div_ceil(elem).max(1);
        let mut buffer: Vec<PdhFmtCounterValueItem> = Vec::with_capacity(capacity);
        let mut alloc_size = u32::try_from(capacity.saturating_mul(elem)).unwrap_or(buffer_size);
        // SAFETY: `buffer` holds `capacity` correctly aligned items totalling
        // `alloc_size` bytes (>= the sizing pass), enough for the items plus the
        // name strings PDH appends; `self.counter` and the out-params are valid.
        if unsafe {
            PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &raw mut alloc_size,
                &raw mut item_count,
                buffer.as_mut_ptr(),
            )
        } != PDH_SUCCESS
        {
            return None;
        }

        // SAFETY: PDH initialized `item_count` contiguous items at the front of
        // `buffer`; the iteration below reads no further than that.
        let items = unsafe { std::slice::from_raw_parts(buffer.as_ptr(), item_count as usize) };
        let sum: f64 = items
            .iter()
            .filter(|item| matches!(item.value.c_status, PDH_SUCCESS | PDH_CSTATUS_NEW_DATA))
            .map(|item| item.value.double_value)
            .sum();
        Some(rounded_percent(sum))
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code, reason = "PDH FFI for the GPU engine counters")]
impl Drop for GpuQuery {
    fn drop(&mut self) {
        // SAFETY: `self.query` was opened by PdhOpenQueryW and is closed once.
        unsafe { PdhCloseQuery(self.query) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_mean_first_sample_is_undiluted() {
        let mut mean = RollingMean::default();
        assert!((mean.push(20.0) - 20.0).abs() < f32::EPSILON);
        assert!((mean.push(10.0) - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rolling_mean_evicts_the_oldest_sample() {
        let mut mean = RollingMean::default();
        // Fill the window with zeros, then push spikes: once the window
        // holds only the spikes, the zeros no longer drag the mean down.
        for _ in 0..CPU_SMOOTHING_WINDOW_POLLS {
            mean.push(0.0);
        }
        let mut value = 0.0;
        for _ in 0..CPU_SMOOTHING_WINDOW_POLLS {
            value = mean.push(50.0);
        }
        assert!((value - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filled_cells_rounds_up_per_ten_percent_bucket() {
        assert_eq!(filled_cells(0), 0);
        assert_eq!(filled_cells(1), 1);
        assert_eq!(filled_cells(10), 1);
        assert_eq!(filled_cells(11), 2);
        assert_eq!(filled_cells(100), 10);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_drm_fdinfo_reads_engine_busy_ns_and_skips_capacity() {
        let input = "drm-driver:\tamdgpu\n\
                     drm-client-id:\t42\n\
                     drm-engine-gfx:\t1500 ns\n\
                     drm-engine-capacity-gfx:\t2\n\
                     drm-engine-compute:\t250 ns\n";
        assert_eq!(
            parse_drm_fdinfo(input),
            Some(DrmClientSample {
                client_id:      Some(42),
                engine_busy_ns: vec![("gfx".to_string(), 1500), ("compute".to_string(), 250),],
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_drm_fdinfo_returns_none_without_engine_lines() {
        // A render-node fd on a driver that emits no engine utilization
        // (the Apple `asahi` driver looks exactly like this).
        let input = "pos:\t0\nflags:\t02400002\nmnt_id:\t39\nino:\t460\n";
        assert_eq!(parse_drm_fdinfo(input), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn engine_busy_percent_is_busy_over_window() {
        assert_eq!(engine_busy_percent(500_000_000, 1_000_000_000), Some(50));
        assert_eq!(engine_busy_percent(2_000_000_000, 1_000_000_000), Some(100));
        assert_eq!(engine_busy_percent(0, 1_000_000_000), Some(0));
        assert_eq!(engine_busy_percent(10, 0), None);
    }

    #[test]
    fn spawned_monitor_reports_sampling_with_a_connected_receiver() {
        // The event loop gates `Select` registration on `is_sampling()`.
        // A spawned worker holds the sample sender for the monitor's life,
        // so the receiver is connected (try_recv is Empty, not
        // Disconnected) and registering it will not busy-spin.
        let monitor = CpuMonitor::new(1000);
        assert!(monitor.is_sampling());
        assert!(matches!(
            monitor.receiver().try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        ));
    }
}

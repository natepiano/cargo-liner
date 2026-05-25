//! Sysinfo-backed CPU/GPU sampler plus severity buckets keyed to the
//! framework's theme colors.
//!
//! [`CpuPoller`] holds a sysinfo [`System`] handle and the polling
//! cadence; [`poll_if_due`](CpuPoller::poll_if_due) returns a fresh
//! [`CpuUsage`] only when the configured interval has elapsed.
//! [`severity`] maps a percentage to a [`CpuSeverity`] bucket using
//! caller-supplied thresholds; [`CpuSeverity::color`] resolves to the
//! framework's success / title / error theme colors.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

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

/// Aggregate CPU/GPU sample returned by [`CpuPoller::poll_if_due`].
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

/// Sysinfo-backed CPU/GPU sampler that rate-limits polls to the
/// configured interval.
#[derive(Debug)]
pub struct CpuPoller {
    system:             System,
    last_poll:          Option<Instant>,
    poll_interval:      Duration,
    last_breakdown_raw: CpuBreakdownRaw,
    /// Persistent PDH query for GPU utilization (Windows only).
    #[cfg(target_os = "windows")]
    gpu_query:          Option<GpuQuery>,
}

impl CpuPoller {
    /// Construct a poller that refreshes at most every
    /// `poll_interval_ms` milliseconds.
    #[must_use]
    pub fn new(poll_interval_ms: u64) -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );
        system.refresh_cpu_all();
        Self {
            system,
            last_poll: None,
            poll_interval: Duration::from_millis(poll_interval_ms),
            last_breakdown_raw: read_cpu_breakdown_raw(),
            #[cfg(target_os = "windows")]
            gpu_query: GpuQuery::new(),
        }
    }

    /// Number of CPU cores reported by the underlying [`System`], floored at 1.
    #[must_use]
    pub fn core_count(&self) -> usize { self.system.cpus().len().max(1) }

    /// Zero-filled [`CpuUsage`] sized to the current core count.
    #[must_use]
    pub fn placeholder_cpu_usage(&self) -> CpuUsage { CpuUsage::placeholder(self.core_count()) }

    /// Return a fresh sample if at least `poll_interval` has elapsed
    /// since the previous poll, otherwise `None`.
    pub fn poll_if_due(&mut self, now: Instant) -> Option<CpuUsage> {
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < self.poll_interval)
        {
            return None;
        }

        self.last_poll = Some(now);

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
        #[cfg(not(target_os = "windows"))]
        let gpu_percent = read_gpu_percent();

        let usage = CpuUsage {
            total_percent,
            cores,
            breakdown,
            gpu_percent,
        };
        Some(usage)
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

#[cfg(target_os = "macos")]
fn read_gpu_percent() -> Option<u8> {
    let output = Command::new("ioreg")
        .args(["-r", "-d", "1", "-w0", "-c", "IOAccelerator"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gpu_percent(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
#[cfg(target_os = "linux")]
fn read_gpu_percent() -> Option<u8> { linux_sysfs_gpu_percent().or_else(linux_nvidia_gpu_percent) }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_gpu_percent() -> Option<u8> { None }

#[cfg(target_os = "macos")]
fn parse_gpu_percent(output: &str) -> Option<u8> {
    let needle = "\"Device Utilization %\"=";
    let after = output.split_once(needle)?.1.trim_start();
    let digits = after
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse::<u8>().ok().map(|value| value.min(100))
}

#[cfg(target_os = "macos")]
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
    const KERN_SUCCESS: KernReturn = 0;

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

#[cfg(target_os = "windows")]
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
    fn filled_cells_rounds_up_per_ten_percent_bucket() {
        assert_eq!(filled_cells(0), 0);
        assert_eq!(filled_cells(1), 1);
        assert_eq!(filled_cells(10), 1);
        assert_eq!(filled_cells(11), 2);
        assert_eq!(filled_cells(100), 10);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_gpu_percent_finds_device_utilization() {
        let input =
            r#""PerformanceStatistics" = {"Renderer Utilization %"=10,"Device Utilization %"=42}"#;
        assert_eq!(parse_gpu_percent(input), Some(42));
    }
}

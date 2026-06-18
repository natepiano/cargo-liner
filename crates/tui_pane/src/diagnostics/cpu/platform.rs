#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::time::Instant;

#[cfg(target_os = "macos")]
use super::CFDictionary;
#[cfg(target_os = "macos")]
use super::CFNumber;
#[cfg(target_os = "macos")]
use super::CFRetained;
#[cfg(target_os = "macos")]
use super::CFString;
#[cfg(target_os = "macos")]
use super::CFType;
use super::CpuBreakdownRaw;
#[cfg(target_os = "windows")]
use super::GPU_COUNTER_PATH;
use super::GpuUsage;
#[cfg(target_os = "macos")]
use super::IOIteratorNext;
#[cfg(target_os = "macos")]
use super::IOObjectRelease;
#[cfg(target_os = "macos")]
use super::IORegistryEntryCreateCFProperty;
#[cfg(target_os = "macos")]
use super::IOServiceGetMatchingServices;
#[cfg(target_os = "macos")]
use super::IOServiceMatching;
#[cfg(target_os = "macos")]
use super::KERN_SUCCESS;
#[cfg(target_os = "windows")]
use super::PDH_CSTATUS_NEW_DATA;
#[cfg(target_os = "windows")]
use super::PDH_FMT_DOUBLE;
#[cfg(target_os = "windows")]
use super::PDH_MORE_DATA;
#[cfg(target_os = "windows")]
use super::PDH_SUCCESS;
use super::bounded_percent_u8;
#[cfg(target_os = "macos")]
use super::from_ref;
#[cfg(target_os = "macos")]
use super::io_iterator_t;
#[cfg(target_os = "macos")]
use super::kCFAllocatorDefault;
#[cfg(target_os = "macos")]
use super::kIOMainPortDefault;
#[cfg(target_os = "windows")]
use super::rounded_percent;

#[cfg(target_os = "macos")]
pub(super) fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { macos_cpu_breakdown_raw() }

#[cfg(target_os = "linux")]
pub(super) fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { linux_cpu_breakdown_raw() }

#[cfg(target_os = "windows")]
pub(super) fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { windows_cpu_breakdown_raw() }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(super) fn read_cpu_breakdown_raw() -> CpuBreakdownRaw { CpuBreakdownRaw::default() }

/// Query the I/O Registry directly for the first `IOAccelerator` service
/// whose properties report GPU metrics. Replaces spawning `ioreg` on every
/// poll.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "IOKit FFI replaces the per-poll ioreg spawn")]
pub(super) fn read_gpu_usage() -> GpuUsage {
    // SAFETY: the argument is a valid NUL-terminated C string the call
    // copies into the returned matching dictionary.
    let Some(matching) = (unsafe { IOServiceMatching(c"IOAccelerator".as_ptr()) }) else {
        return GpuUsage::default();
    };
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
        return GpuUsage::default();
    }

    let mut gpu_usage = GpuUsage::default();
    loop {
        let accelerator = IOIteratorNext(services);
        if accelerator == 0 {
            break;
        }
        let core_count = gpu_core_count(accelerator);
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
        let mut usage = GpuUsage {
            core_count,
            ..GpuUsage::default()
        };
        if let Some(statistics) = statistics
            && let Ok(statistics) = statistics.downcast::<CFDictionary>()
        {
            usage.device_percent = statistics_percent(&statistics, "Device Utilization %");
            usage.renderer_percent = statistics_percent(&statistics, "Renderer Utilization %");
            usage.tiler_percent = statistics_percent(&statistics, "Tiler Utilization %");
        }
        if usage != GpuUsage::default() {
            gpu_usage = usage;
            break;
        }
    }
    IOObjectRelease(services);
    gpu_usage
}

#[cfg(not(target_os = "macos"))]
#[cfg(target_os = "linux")]
pub(super) fn read_gpu_usage() -> GpuUsage {
    GpuUsage::from_device_percent(linux_sysfs_gpu_percent().or_else(linux_nvidia_gpu_percent))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(super) fn read_gpu_usage() -> GpuUsage { GpuUsage::default() }

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "CoreFoundation dictionary FFI for the GPU statistics lookup"
)]
fn gpu_core_count(accelerator: u32) -> Option<u16> {
    // SAFETY: `accelerator` is a live service handle from `IOIteratorNext`;
    // the key outlives the call and the retained result is consumed locally.
    let value = unsafe {
        IORegistryEntryCreateCFProperty(
            accelerator,
            Some(&CFString::from_static_str("gpu-core-count")),
            kCFAllocatorDefault,
            0,
        )
    }?;
    let number = value.downcast::<CFNumber>().ok()?;
    let count = u64::try_from(number.as_i64()?).ok()?;
    u16::try_from(count).ok()
}

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "CoreFoundation dictionary FFI for the GPU statistics lookup"
)]
fn statistics_percent(statistics: &CFDictionary, key: &'static str) -> Option<u8> {
    let key = CFString::from_static_str(key);
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
pub(super) fn macos_cpu_breakdown_raw() -> CpuBreakdownRaw {
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
pub(super) fn linux_cpu_breakdown_raw() -> CpuBreakdownRaw {
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
pub(super) struct DrmClientSample {
    /// `drm-client-id`, used to dedupe a client that holds several fds.
    pub(super) client_id:      Option<i64>,
    /// `(engine name, cumulative busy ns)` per `drm-engine-<name>` line.
    pub(super) engine_busy_ns: Vec<(String, u64)>,
}

/// Parse the `drm-` usage-stats keys from one `/proc/<pid>/fdinfo/<fd>`
/// file. Returns `None` for any fd that exposes no `drm-engine-*` lines:
/// non-DRM fds, and DRM drivers that emit no engine utilization (the
/// Apple `asahi` driver among them).
#[cfg(target_os = "linux")]
pub(super) fn parse_drm_fdinfo(contents: &str) -> Option<DrmClientSample> {
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
pub(super) struct FdinfoGpuSampler {
    previous_busy_ns:    HashMap<String, u64>,
    previous_sampled_at: Instant,
}

#[cfg(target_os = "linux")]
impl FdinfoGpuSampler {
    /// Prime the baseline from the current engine totals.
    pub(super) fn new() -> Self {
        Self {
            previous_busy_ns:    collect_drm_engine_busy_ns(),
            previous_sampled_at: Instant::now(),
        }
    }

    /// Sample the busiest engine's utilization since the previous poll,
    /// in `0..=100`. Returns `None` when no DRM client exposes engine
    /// stats (this driver provides no `fdinfo` utilization).
    pub(super) fn sample(&mut self) -> Option<u8> {
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
pub(super) fn engine_busy_percent(busy_ns: u64, elapsed_ns: u128) -> Option<u8> {
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
pub(super) fn windows_cpu_breakdown_raw() -> CpuBreakdownRaw {
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
pub(super) struct GpuQuery {
    query:   isize,
    counter: isize,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code, reason = "PDH FFI for the GPU engine counters")]
impl GpuQuery {
    /// Open the query, add the wildcard counter, and prime the baseline
    /// sample. Returns `None` if PDH or the GPU counter is unavailable.
    pub(super) fn new() -> Option<Self> {
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
    pub(super) fn sample(&self) -> Option<u8> {
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

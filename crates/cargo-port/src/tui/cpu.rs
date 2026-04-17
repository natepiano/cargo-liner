use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use ratatui::style::Color;
use sysinfo::CpuRefreshKind;
use sysinfo::RefreshKind;
use sysinfo::System;

use super::constants::ERROR_COLOR;
use super::constants::INACTIVE_BORDER_COLOR;
use super::constants::SUCCESS_COLOR;
use super::constants::TITLE_COLOR;
use crate::config::CpuConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CpuCoreSnapshot {
    pub label:   String,
    pub percent: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CpuSnapshot {
    pub total_percent: u8,
    pub cores:         Vec<CpuCoreSnapshot>,
    pub breakdown:     CpuBreakdownSnapshot,
    pub gpu_percent:   Option<u8>,
}

impl CpuSnapshot {
    pub(super) fn placeholder(core_count: usize) -> Self {
        Self {
            total_percent: 0,
            cores:         (0..core_count)
                .map(|index| CpuCoreSnapshot {
                    label:   format!("CPU {}", index + 1),
                    percent: 0,
                })
                .collect(),
            breakdown:     CpuBreakdownSnapshot::default(),
            gpu_percent:   read_gpu_percent(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CpuBreakdownSnapshot {
    pub system_percent: u8,
    pub user_percent:   u8,
    pub idle_percent:   u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CpuSeverity {
    Green,
    Yellow,
    Red,
}

impl CpuSeverity {
    pub(super) const fn color(self) -> Color {
        match self {
            Self::Green => SUCCESS_COLOR,
            Self::Yellow => TITLE_COLOR,
            Self::Red => ERROR_COLOR,
        }
    }
}

#[derive(Debug)]
pub(super) struct CpuPoller {
    system:             System,
    last_poll:          Option<Instant>,
    poll_interval:      Duration,
    last_breakdown_raw: CpuBreakdownRaw,
}

impl CpuPoller {
    pub(super) fn new(config: &CpuConfig) -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );
        system.refresh_cpu_all();
        Self {
            system,
            last_poll: None,
            poll_interval: Duration::from_millis(config.poll_ms),
            last_breakdown_raw: read_cpu_breakdown_raw(),
        }
    }

    pub(super) fn core_count(&self) -> usize { self.system.cpus().len().max(1) }

    pub(super) fn placeholder_snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::placeholder(self.core_count())
    }

    pub(super) fn poll_if_due(&mut self, now: Instant) -> Option<CpuSnapshot> {
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
            .map(|(index, cpu)| CpuCoreSnapshot {
                label:   normalize_cpu_label(cpu.name(), index),
                percent: cpu_percent(cpu.cpu_usage()),
            })
            .collect::<Vec<_>>();

        let total_percent = cpu_percent(self.system.global_cpu_usage());
        let snapshot = CpuSnapshot {
            total_percent,
            cores,
            breakdown: cpu_breakdown(&mut self.last_breakdown_raw),
            gpu_percent: read_gpu_percent(),
        };
        Some(snapshot)
    }
}

pub(super) fn filled_cells(percent: u8) -> usize {
    let clamped = if percent > 100 { 100 } else { percent };
    usize::from(clamped).div_ceil(10)
}

pub(super) fn severity(percent: u8, config: &CpuConfig) -> CpuSeverity {
    if percent <= config.green_max_percent {
        CpuSeverity::Green
    } else if percent <= config.yellow_max_percent {
        CpuSeverity::Yellow
    } else {
        CpuSeverity::Red
    }
}

pub(super) const fn blank_bar_color() -> Color { INACTIVE_BORDER_COLOR }

fn cpu_percent(value: f32) -> u8 {
    let rounded = value.round().clamp(0.0, 100.0);
    rounded as u8
}

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

fn cpu_breakdown(previous: &mut CpuBreakdownRaw) -> CpuBreakdownSnapshot {
    let current = read_cpu_breakdown_raw();
    let delta_system = current.system.saturating_sub(previous.system);
    let delta_user = current.user.saturating_sub(previous.user);
    let delta_idle = current.idle.saturating_sub(previous.idle);
    let delta_total = delta_system
        .saturating_add(delta_user)
        .saturating_add(delta_idle);
    *previous = current;

    if delta_total == 0 {
        return CpuBreakdownSnapshot::default();
    }

    CpuBreakdownSnapshot {
        system_percent: percent_from_parts(delta_system, delta_total),
        user_percent:   percent_from_parts(delta_user, delta_total),
        idle_percent:   percent_from_parts(delta_idle, delta_total),
    }
}

fn percent_from_parts(value: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let percent = ((value as f64 / total as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0);
    percent as u8
}

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

#[cfg(target_os = "windows")]
fn read_gpu_percent() -> Option<u8> { windows_gpu_percent() }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_gpu_percent() -> Option<u8> { None }

fn parse_gpu_percent(output: &str) -> Option<u8> {
    let needle = "\"Device Utilization %\"=";
    let after = output.split_once(needle)?.1.trim_start();
    let digits = after
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
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
    let mut count =
        (std::mem::size_of::<HostCpuLoadInfo>() / std::mem::size_of::<Integer>()) as MachMsgCount;

    let result = unsafe {
        host_statistics(
            mach_host_self(),
            HOST_CPU_LOAD_INFO,
            (&mut info as *mut HostCpuLoadInfo).cast::<Integer>(),
            &mut count,
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

    let ok = unsafe { GetSystemTimes(&mut idle_time, &mut kernel_time, &mut user_time) };
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

#[cfg(target_os = "windows")]
fn windows_gpu_percent() -> Option<u8> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-Counter '\\GPU Engine(*engtype_3D)\\Utilization Percentage').CounterSamples | Measure-Object -Property CookedValue -Sum | Select-Object -ExpandProperty Sum",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    Some(value.round().clamp(0.0, 100.0) as u8)
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

    #[test]
    fn parse_gpu_percent_finds_device_utilization() {
        let input =
            r#""PerformanceStatistics" = {"Renderer Utilization %"=10,"Device Utilization %"=42}"#;
        assert_eq!(parse_gpu_percent(input), Some(42));
    }
}

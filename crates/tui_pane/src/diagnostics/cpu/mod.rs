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

pub use super::constants::CPU_SMOOTHING_WINDOW_POLLS;
#[cfg(target_os = "windows")]
use super::constants::GPU_COUNTER_PATH;
#[cfg(target_os = "macos")]
use super::constants::KERN_SUCCESS;
#[cfg(target_os = "windows")]
use super::constants::PDH_CSTATUS_NEW_DATA;
#[cfg(target_os = "windows")]
use super::constants::PDH_FMT_DOUBLE;
#[cfg(target_os = "windows")]
use super::constants::PDH_MORE_DATA;
#[cfg(target_os = "windows")]
use super::constants::PDH_SUCCESS;
use super::constants::PERCENT_PER_CELL;
use crate::theme;

mod monitor;
mod percent;
mod platform;
mod poller;
mod rolling_mean;
mod severity;
mod types;

pub use monitor::CpuMonitor;
use percent::CpuBreakdownRaw;
use percent::bounded_percent_u8;
use percent::cpu_breakdown;
use percent::cpu_percent;
use percent::normalize_cpu_label;
use platform::read_cpu_breakdown_raw;
use platform::read_gpu_percent;
pub use poller::CpuPoller;
pub use rolling_mean::RollingMean;
pub use severity::CpuSeverity;
pub use severity::blank_bar_color;
pub use severity::filled_cells;
pub use severity::severity;
pub use types::CpuBreakdown;
pub use types::CpuCoreUsage;
pub use types::CpuUsage;

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

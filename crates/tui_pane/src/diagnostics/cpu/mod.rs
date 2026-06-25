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

use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::ptr::from_ref;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

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

pub use monitor::CpuMonitor;
use percent::CpuBreakdownRaw;
use percent::bounded_percent_u8;
use percent::cpu_breakdown;
use percent::cpu_percent;
use percent::normalize_cpu_label;
#[cfg(target_os = "windows")]
use percent::rounded_percent;
#[cfg(all(test, target_os = "linux"))]
use platform::DrmClientSample;
#[cfg(target_os = "linux")]
use platform::FdinfoGpuSampler;
#[cfg(target_os = "windows")]
use platform::GpuQuery;
#[cfg(all(test, target_os = "linux"))]
use platform::engine_busy_percent;
#[cfg(all(test, target_os = "linux"))]
use platform::parse_drm_fdinfo;
use platform::read_cpu_breakdown_raw;
#[cfg(not(target_os = "windows"))]
use platform::read_gpu_usage;
pub use poller::CpuBreakdown;
pub use poller::CpuCoreUsage;
pub use poller::CpuPoller;
pub use poller::CpuUsage;
pub use poller::GpuUsage;
pub use rolling_mean::RollingMean;
pub use severity::CpuSeverity;
pub use severity::blank_bar_color;
pub use severity::filled_cells;
pub use severity::severity;

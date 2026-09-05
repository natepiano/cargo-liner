//! The window-server calls behind [`Desktop`](super::Desktop), one
//! module per platform.
//!
//! Exactly one backend is compiled, and both answer the same five
//! questions, so everything above this module is written once.

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use self::fallback::capture;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use self::fallback::window_at;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use self::fallback::window_frame;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use self::fallback::window_titled;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use self::fallback::window_titles;
#[cfg(target_os = "linux")]
pub(super) use self::linux::capture;
#[cfg(target_os = "linux")]
pub(super) use self::linux::window_at;
#[cfg(target_os = "linux")]
pub(super) use self::linux::window_frame;
#[cfg(target_os = "linux")]
pub(super) use self::linux::window_titled;
#[cfg(target_os = "linux")]
pub(super) use self::linux::window_titles;
#[cfg(target_os = "macos")]
pub(super) use self::macos::capture;
#[cfg(target_os = "macos")]
pub(super) use self::macos::window_at;
#[cfg(target_os = "macos")]
pub(super) use self::macos::window_frame;
#[cfg(target_os = "macos")]
pub(super) use self::macos::window_titled;
#[cfg(target_os = "macos")]
pub(super) use self::macos::window_titles;

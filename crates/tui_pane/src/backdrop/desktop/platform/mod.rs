//! The window-server calls behind [`Desktop`](super::Desktop), one
//! module per platform.
//!
//! Exactly one backend is compiled, and both answer the same five
//! questions, so everything above this module is written once.

#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use self::fallback::capture;
#[cfg(not(target_os = "macos"))]
pub(super) use self::fallback::window_at;
#[cfg(not(target_os = "macos"))]
pub(super) use self::fallback::window_frame;
#[cfg(not(target_os = "macos"))]
pub(super) use self::fallback::window_titled;
#[cfg(not(target_os = "macos"))]
pub(super) use self::fallback::window_titles;
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

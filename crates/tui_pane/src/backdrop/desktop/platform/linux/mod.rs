//! The KDE Wayland backend.
//!
//! `kdotool` supplies `KWin` window UUIDs, `KWin` supplies current window geometry over D-Bus, and
//! `kscreen-doctor` supplies the logical output layout.
//!
//! What stands underneath the terminal is assembled rather than photographed: [`compose`] reads
//! `KWin`'s stacking order and draws every window below this one over the display's own desktop
//! window, so what the animation is drawn from is the monitor as it stands, other windows and all,
//! without this terminal in it. Where no picture can be taken -- `KWin` refusing the screenshot
//! interface, a layout that moved under the read, a stack this window is not in -- Plasma's
//! wallpaper configuration is read over D-Bus and rendered at the selected output's coordinates
//! instead, which is what this backend did for every capture before.

mod cached_wallpaper;
mod capture;
mod compose;
mod constants;
mod desktop_subprocess;
mod display;
mod held_composite;
mod session_connection;
mod wallpaper;
mod window;

pub(in crate::backdrop::desktop) use self::capture::capture;
pub use self::compose::kwin_evaluate;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::TerminalWindowSearchOutcome;
use crate::backdrop::desktop::TitledWindow;

/// See [`crate::backdrop::desktop::window_frame`].
pub(in crate::backdrop::desktop) fn window_frame(window: u32) -> Option<Frame> {
    self::window::frame(window)
}

/// See [`crate::backdrop::desktop::window_titles`].
pub(in crate::backdrop::desktop) fn window_titles() -> Vec<TitledWindow> { self::window::titles() }

/// See [`crate::backdrop::desktop::window_titled`].
pub(in crate::backdrop::desktop) fn window_titled(marker: &str) -> TerminalWindowSearchOutcome {
    self::window::titled(marker)
}

/// See [`crate::backdrop::desktop::window_at`].
pub(in crate::backdrop::desktop) fn window_at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    self::window::at(origin)
}

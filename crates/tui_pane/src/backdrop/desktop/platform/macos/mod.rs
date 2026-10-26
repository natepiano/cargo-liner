//! The macOS capture backend.
//!
//! A persistent `ScreenCaptureKit` stream -- one per display, in the [`stream`] submodule --
//! captures the desktop on the selected terminal window's display, and `CoreGraphics` answers the
//! per-window questions -- where a window stands, what it is titled -- that the drawing threads ask
//! far more often.

mod capture;
mod constants;
mod display;
mod stream;
mod window;

pub(in crate::backdrop::desktop) use self::capture::capture;
use self::constants::BYTES_PER_PIXEL;
pub(in crate::backdrop::desktop) use self::window::window_at;
pub(in crate::backdrop::desktop) use self::window::window_frame;
pub(in crate::backdrop::desktop) use self::window::window_titled;
pub(in crate::backdrop::desktop) use self::window::window_titles;

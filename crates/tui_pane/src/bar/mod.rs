//! Bar primitives + framework bar renderer.
//!
//! The `bar` module owns the public bar surface: leaf primitives
//! ([`BarRegion`], [`BarSlot`], [`ShortcutState`], [`Visibility`])
//! plus [`render`] / [`StatusBar`] — the renderer that the binary
//! drives once per frame.
//!
//! The renderer's contract:
//!
//! 1. Resolve `pane_slots: Vec<RenderedSlot>` for the focused pane. Overlay-first dispatch (Keymap
//!    / Settings overlays read `framework.{keymap,settings}_pane.bar_slots()`); else
//!    [`FocusedPane::App(id)`](crate::FocusedPane::App) flows through
//!    [`Keymap::render_app_pane_bar_slots`](crate::Keymap::render_app_pane_bar_slots); else
//!    [`FocusedPane::Framework(FrameworkFocusId::Toasts)`](crate::FocusedPane::Framework) reads
//!    from `framework.toasts.bar_slots(ctx)`.
//! 2. Walk [`BarRegion::ALL`](crate::BarRegion::ALL) and dispatch to each region module. Each
//!    module owns its own suppression rule based on
//!    [`Framework::focused_pane_mode`](crate::Framework::focused_pane_mode).
//! 3. Concatenate the per-region span vectors into one [`StatusBar`].

mod constants;
mod global_region;
mod nav_region;
mod palette;
mod pane_action_region;
mod region;
mod render;
mod slot;
mod status_bar;
mod status_line;
mod support;
mod visibility;

pub use palette::BarPalette;
pub use region::BarRegion;
pub use render::render;
pub use slot::BarSlot;
pub use slot::ShortcutState;
pub use status_bar::StatusBar;
pub use status_line::ScanIndicator;
pub use status_line::StatusLine;
pub use status_line::StatusLineGlobal;
pub use status_line::render as render_status_line;
pub use status_line::status_line_global_spans;
pub use visibility::Visibility;

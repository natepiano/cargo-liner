//! Framework-owned panes: built-in overlays and the toast stack.
//!
//! Three structs live here, all generic over `Ctx: AppContext`:
//! - [`KeymapPane<Ctx>`]: keymap viewer/editor overlay.
//! - [`SettingsPane<Ctx>`]: settings overlay.
//! - [`Toasts<Ctx>`]: transient notification stack (focusable when active).
//!
//! These ship inherent methods rather than implementing
//! [`Pane<Ctx>`](crate::Pane) / [`Shortcuts<Ctx>`](crate::Shortcuts):
//! those traits require a [`Self::APP_PANE_ID`](crate::Pane::APP_PANE_ID),
//! and framework panes carry [`FrameworkOverlayId`](crate::FrameworkOverlayId)
//! / [`FrameworkFocusId`](crate::FrameworkFocusId) instead. The bar
//! renderer (Phase 13) and input dispatcher (Phase 15) special-case
//! framework panes.

mod keymap_pane;
mod settings_pane;
mod toasts;

pub use keymap_pane::KeymapPane;
pub use keymap_pane::KeymapPaneAction;
pub use settings_pane::SettingsPane;
pub use settings_pane::SettingsPaneAction;
pub use toasts::Toast;
pub use toasts::ToastId;
pub use toasts::ToastStyle;
pub use toasts::Toasts;
pub use toasts::ToastsAction;

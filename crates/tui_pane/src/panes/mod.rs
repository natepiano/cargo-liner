//! Framework-owned panes: built-in overlays and the toast stack.
//!
//! Two overlay structs live here, both generic over `Ctx: AppContext`,
//! plus the toast action vocabulary:
//! - [`KeymapPane<Ctx>`]: keymap viewer/editor overlay.
//! - [`SettingsPane<Ctx>`]: settings overlay.
//!
//! These ship inherent methods rather than implementing
//! [`Pane<Ctx>`](crate::Pane) / [`Shortcuts<Ctx>`](crate::Shortcuts):
//! those traits require a [`Self::APP_PANE_ID`](crate::Pane::APP_PANE_ID),
//! and framework panes carry [`FrameworkOverlayId`](crate::FrameworkOverlayId)
//! / [`FrameworkFocusId`](crate::FrameworkFocusId) instead. The bar
//! renderer (Phase 13) and input dispatcher (Phase 15) special-case
//! framework panes.

mod keymap;
mod settings;
mod toasts;

pub use keymap::KeymapPane;
pub use keymap::KeymapPaneAction;
pub use settings::SettingsCommand;
pub use settings::SettingsPane;
pub use settings::SettingsPaneAction;
pub use settings::SettingsRender;
pub use settings::SettingsRenderOptions;
pub use toasts::ToastsAction;

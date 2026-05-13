//! Framework-owned panes: built-in overlays.
//!
//! Two overlay structs live here:
//! - [`KeymapPane`]: keymap viewer/editor overlay.
//! - [`SettingsPane`]: settings overlay.
//!
//! These ship inherent methods rather than implementing
//! [`Pane<Ctx>`](crate::Pane) / [`Shortcuts<Ctx>`](crate::Shortcuts):
//! those traits require a [`Self::APP_PANE_ID`](crate::Pane::APP_PANE_ID),
//! and framework panes carry [`FrameworkOverlayId`](crate::FrameworkOverlayId)
//! / [`FrameworkFocusId`](crate::FrameworkFocusId) instead. The bar
//! renderer and input dispatcher special-case framework panes.

mod keymap;
mod settings;

pub use keymap::KeymapCaptureCommand;
pub use keymap::KeymapPane;
pub use keymap::KeymapPaneAction;
pub use settings::SettingsCommand;
pub use settings::SettingsPane;
pub use settings::SettingsPaneAction;
pub use settings::SettingsRender;
pub use settings::SettingsRenderOptions;

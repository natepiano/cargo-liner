//! Framework-owned panes: built-in overlays.
//!
//! Three overlay structs live here:
//! - [`KeymapPane`]: keymap viewer/editor overlay.
//! - [`SettingsPane`]: settings overlay.
//! - [`GlobalShortcutsPane`]: read-only global shortcut viewer.
//!
//! Both consume [`OverlayAction`], the single action set for the
//! framework-owned overlay bar (`StartEdit` / `Cancel`). The TOML
//! source for that action set is the shared `[overlay]` table.
//!
//! These ship inherent methods rather than implementing
//! [`Pane<Ctx>`](crate::Pane) / [`Shortcuts<Ctx>`](crate::Shortcuts):
//! those traits require a [`Self::APP_PANE_ID`](crate::Pane::APP_PANE_ID),
//! and framework panes carry [`FrameworkOverlayId`](crate::FrameworkOverlayId)
//! / [`FrameworkFocusId`](crate::FrameworkFocusId) instead. The bar
//! renderer and input dispatcher special-case framework panes.

mod constants;
mod global_shortcuts;
mod keymap;
mod keymap_ui;
mod settings;

crate::action_enum! {
    /// Actions reachable on a framework overlay's local bar.
    ///
    /// Shared by [`KeymapPane`] and [`SettingsPane`]. The TOML overlay
    /// source for both panes is the single `[overlay]` table.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum OverlayAction {
        /// Begin editing the selected row.
        StartEdit => ("start_edit", "edit",   "Edit selected row");
        /// Discard pending edits and close.
        Cancel    => ("cancel",     "cancel", "Cancel");
    }
}

pub use global_shortcuts::GlobalShortcutsPane;
pub use keymap::KeymapCaptureCommand;
pub use keymap::KeymapPane;
pub use keymap_ui::KEYMAP_POPUP_MAX_HEIGHT;
pub use keymap_ui::KeymapOverlayInputs;
pub use keymap_ui::KeymapUiContext;
pub use settings::SettingsCommand;
pub use settings::SettingsPane;
pub use settings::SettingsRenderOptions;

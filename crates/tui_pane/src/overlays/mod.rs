//! Framework-owned panes: built-in overlays.
//!
//! Three overlay structs live here:
//! - [`KeymapPane`]: keymap viewer/editor overlay.
//! - [`SettingsPane`]: settings overlay.
//! - [`GlobalShortcutsPane`]: selectable global shortcut viewer.
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
mod keymap_edit;
mod keymap_ui;
mod settings;

/// How wide `line` draws, counting what each character actually
/// occupies rather than how many there are.
///
/// What the overlays size their popups by. A popup measured in
/// `char`s is too narrow wherever a description holds anything wide,
/// and too wide wherever one holds a combining mark.
fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

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
pub use keymap_edit::KeymapEditContext;
pub use keymap_edit::dispatch_keymap_action;
pub use keymap_edit::edit_selected_global_shortcut;
pub use keymap_edit::handle_keymap_capture_command;
pub use keymap_edit::handle_keymap_navigation_key;
pub use keymap_edit::keymap_toml;
pub use keymap_edit::save_keymap_to_disk;
pub use keymap_ui::KEYMAP_POPUP_MAX_HEIGHT;
pub use keymap_ui::KeymapOverlayInputs;
pub use keymap_ui::KeymapUiContext;
use ratatui::text::Line;
pub use settings::SettingsCommand;
pub use settings::SettingsPane;
pub use settings::SettingsRenderOptions;
use unicode_width::UnicodeWidthStr as _;

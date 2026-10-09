//! The framework overlay drawn after an app's panes, toasts and modal.

use ratatui::Frame;

use super::global_shortcuts;
use super::keymap_ui;
use super::keymap_ui::KeymapUiContext;
use crate::FrameworkOverlayId;
use crate::Keymap;
use crate::SettingsRows;
use crate::draw_settings;

/// Draw whichever framework overlay is open: settings with the app's
/// rows, the keymap overlay, or the `?` popup.
///
/// `settings_rows` is called only while settings is open.
pub fn draw_framework_overlay<A: KeymapUiContext + 'static, S: Copy>(
    frame: &mut Frame,
    app: &mut A,
    keymap: &Keymap<A>,
    settings_rows: impl FnOnce(&A) -> SettingsRows<S>,
) {
    match app.framework().overlay() {
        Some(FrameworkOverlayId::Settings) => {
            let rows = settings_rows(app);
            draw_settings(frame, &mut app.framework_mut().settings_pane, &rows);
        },
        Some(FrameworkOverlayId::Keymap) => keymap_ui::draw_keymap_overlay(frame, app, keymap),
        Some(FrameworkOverlayId::GlobalShortcuts) => {
            global_shortcuts::draw_global_shortcuts_overlay(frame, app, keymap);
        },
        None => (),
    }
}

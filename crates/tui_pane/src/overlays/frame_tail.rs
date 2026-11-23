//! The framework overlays drawn after an app's panes.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::global_shortcuts;
use super::keymap_ui;
use super::keymap_ui::KeymapUiContext;
use crate::AppContext;
use crate::Framework;
use crate::FrameworkOverlayId;
use crate::Keymap;
use crate::PaneFocusState;
use crate::Renderable;
use crate::SettingsRows;
use crate::ToastsRenderCtx;

/// Draw the framework's toasts inside `area`.
pub fn render_toasts<Ctx: AppContext>(
    frame: &mut Frame,
    framework: &mut Framework<Ctx>,
    rect: Rect,
) {
    Renderable::render(&mut framework.toasts, frame, rect, &frame_toasts_ctx());
}

/// Draw the framework's toasts over a tile grid in `viewport`. Keep the cards
/// out of the feet of `protected_frames`, and keep a card in place while grid
/// cells change height.
pub fn render_toasts_over_grid<Ctx: AppContext>(
    frame: &mut Frame,
    framework: &mut Framework<Ctx>,
    viewport: Rect,
    protected_frames: &[Rect],
) {
    framework
        .toasts
        .render_over_grid(frame, viewport, protected_frames, &frame_toasts_ctx());
}

/// The `ToastsRenderCtx` for toasts drawn after the app's panes: now, with no pane focus.
fn frame_toasts_ctx() -> ToastsRenderCtx {
    ToastsRenderCtx {
        now:              Instant::now(),
        pane_focus_state: PaneFocusState::Inactive,
    }
}

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
    let area = match app.framework().overlay() {
        Some(FrameworkOverlayId::Settings) => {
            let resolved_settings_rows = settings_rows(app);
            let mode = app.framework().color_mode;
            app.framework_mut()
                .settings_pane
                .draw_popup(frame, &resolved_settings_rows, mode)
        },
        Some(FrameworkOverlayId::Keymap) => keymap_ui::draw_keymap_popup(frame, app, keymap),
        Some(FrameworkOverlayId::GlobalShortcuts) => {
            global_shortcuts::draw_global_shortcuts_popup(frame, app, keymap)
        },
        None => Rect::default(),
    };
    app.framework_mut().set_framework_overlay_bounds(area);
}

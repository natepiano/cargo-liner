mod constants;
mod overlay;
mod pane;
mod render;
mod stats;
mod status_line;

pub(super) use pane::SccachePane;
pub(super) use render::render_sccache_popup;
pub(super) use status_line::SccacheStatusLine;
pub(super) use status_line::apply_summary;
pub(super) use status_line::refresh_summary_if_due;
use tui_pane::CopyLabel;
use tui_pane::CopySelectionResult;
use tui_pane::KeyBind;

use super::app::App;
use super::keymap::SccacheAction;

pub(super) fn open_sccache_stats_overlay(app: &mut App) {
    overlay::open_sccache_stats_overlay(app);
}

pub(super) fn dispatch_sccache_overlay(app: &mut App, bind: &KeyBind) -> bool {
    overlay::dispatch_sccache_overlay(app, bind)
}

/// Resolve the modal stat selection without changing the underlying focus.
pub(super) fn copy_selection(app: &App) -> CopySelectionResult {
    app.overlays
        .sccache_pane
        .selected_copy_value()
        .map_or(CopySelectionResult::Nothing, |target| {
            CopySelectionResult::Payload(tui_pane::CopyPayload::new(
                target.value.clone(),
                CopyLabel::Value,
            ))
        })
}

pub(super) fn dispatch_sccache_action(action: SccacheAction, app: &mut App) {
    match action {
        SccacheAction::Cancel => {
            app.overlays.sccache_pane.pending_chord.clear();
            app.overlays.close_sccache();
        },
    }
}

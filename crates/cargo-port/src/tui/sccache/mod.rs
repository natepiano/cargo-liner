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
use tui_pane::KeyBind;

use super::app::App;

pub(super) fn open_sccache_stats_overlay(app: &mut App) {
    overlay::open_sccache_stats_overlay(app);
}

pub(super) fn dispatch_sccache_overlay(app: &mut App, bind: &KeyBind) -> bool {
    overlay::dispatch_sccache_overlay(app, bind)
}

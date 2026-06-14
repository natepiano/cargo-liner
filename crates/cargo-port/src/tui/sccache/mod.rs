mod constants;
mod overlay;
mod pane;
mod render;
mod stats;

pub(super) use pane::SccachePane;
pub(super) use render::render_sccache_popup;
use tui_pane::KeyBind;

use super::app::App;

pub(super) fn open_sccache_stats_overlay(app: &mut App) {
    overlay::open_sccache_stats_overlay(app);
}

pub(super) fn dispatch_sccache_overlay(app: &mut App, bind: &KeyBind) -> bool {
    overlay::dispatch_sccache_overlay(app, bind)
}

//! The `Pane` trait, the `PaneRenderCtx` bundle, and the
//! `HitboxSink` wrapper used while Phase 10 catches up to
//! `Pane::hit_test`.
//!
//! Phase 8 of the App-API carve (see `docs/app-api.md`). Phase 9
//! reintroduces input/navigation surface (`handle_input`,
//! `is_navigable`) on the trait when the remaining seven panes
//! migrate; pane behavior + size mappings continue to live as
//! `PaneId`-pure free functions in `panes/spec.rs`.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::PaneId;
use crate::tui::config_state::Config;
use crate::tui::interaction::UiHitbox;
use crate::tui::interaction::UiSurface;
use crate::tui::pane::PaneFocusState;
use crate::tui::scan_state::Scan;

/// Hitbox-registration sink used during render. Phase 8.9 wires
/// it up to wrap App's `layout_cache.ui_hitboxes` so per-pane
/// `Pane::render` impls can push hitboxes without `&mut App`.
/// Phase 10 deletes the sink when `Pane::hit_test` becomes a
/// query method.
pub struct HitboxSink<'a> {
    hitboxes: &'a mut Vec<UiHitbox>,
}

impl<'a> HitboxSink<'a> {
    /// Wrap a hitbox vec for push by per-pane render. Constructed
    /// at the dispatch site in `render.rs::render_tiled_pane`.
    pub const fn new(hitboxes: &'a mut Vec<UiHitbox>) -> Self { Self { hitboxes } }

    /// Push a row hitbox for `pane` at `rect` covering row `row`.
    pub fn push_pane_row(&mut self, rect: Rect, pane: PaneId, row: usize, surface: UiSurface) {
        crate::tui::interaction::push_pane_row_hitbox(self.hitboxes, rect, pane, row, surface);
    }
}

/// Bundle of references a pane needs at render time.
pub struct PaneRenderCtx<'a, 'b> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     std::time::Duration,
    pub config:                &'a Config,
    pub scan:                  &'a Scan,
    pub selected_project_path: Option<&'a std::path::Path>,
    pub hit_sink:              &'a mut HitboxSink<'b>,
}

/// Per-pane render dispatch. Phase 9 adds `handle_input` and
/// `is_navigable`; Phase 10 adds `hit_test`.
pub trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitbox_sink_pushes_to_underlying_vec() {
        let mut hitboxes = Vec::new();
        let mut sink = HitboxSink::new(&mut hitboxes);
        sink.push_pane_row(
            ratatui::layout::Rect::new(0, 0, 10, 1),
            PaneId::Cpu,
            3,
            UiSurface::Content,
        );
        assert_eq!(hitboxes.len(), 1);
    }
}

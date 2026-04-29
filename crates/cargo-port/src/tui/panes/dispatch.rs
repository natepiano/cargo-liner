//! The `Pane` trait and the per-pane context bundles
//! (`PaneRenderCtx` / `PaneInputCtx` / `PaneNavCtx`), plus the
//! transitional `HitboxSink` wrapper.
//!
//! Phase 7 of the App-API carve (see `docs/app-api.md`).
//! Introduces the trait + context surface and lays down 13
//! per-pane unit structs that implement the `PaneId`-pure trait
//! methods (`id`, `has_row_hitboxes`, `size_spec`,
//! `input_context`). Render, input, and viewport-access methods
//! land in Phase 8 as each pane absorbs its state and body —
//! during Phase 7 the existing free-function dispatch in
//! `render.rs` and `input.rs` keeps driving the app.
//!
//! The borrow-checker shape (per the design doc) is what forces
//! this thin Phase 7 split: trait method bodies cannot accept
//! `&mut App` while `panes` is mutably borrowed out of App, and
//! the typed ctx bundles cannot be assembled until each pane's
//! per-pane state moves onto its own struct. Phase 8 migrates
//! state + bodies pane-by-pane; the trait method signatures
//! exist now so Phase 8 can fill them in without churning the
//! trait declaration.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use super::PaneId;
use crate::tui::config_state::Config;
use crate::tui::interaction::UiHitbox;
use crate::tui::interaction::UiSurface;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneSizeSpec;

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

/// Routing kind a focused pane reports to App's input router.
///
/// Mirrors today's `InputContext`-after-`PaneBehavior` mapping.
/// Wired into `app/focus.rs::input_context` in Phase 8 once the
/// per-pane impls are reachable from there. Phase 9 deletes
/// `PaneBehavior` once both render and input dispatch flow
/// through the trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Phase 7 foundation; wired into focus.rs in Phase 8"
)]
pub enum InputContextKind {
    ProjectList,
    DetailFields,
    DetailTargets,
    Lints,
    CiRuns,
    Output,
    Toasts,
    Overlay,
}

/// Bundle of references a pane needs at render time. Fields
/// populate pane-by-pane in Phase 8/9 as render bodies migrate.
#[allow(
    dead_code,
    reason = "Phase 8 ctx; subsystem refs added pane-by-pane as bodies migrate"
)]
pub struct PaneRenderCtx<'a, 'b> {
    pub focused_pane:      PaneId,
    pub focus_state:       PaneFocusState,
    pub is_focused:        bool,
    pub animation_elapsed: std::time::Duration,
    pub config:            &'a Config,
    pub hit_sink:          &'a mut HitboxSink<'b>,
}

/// Bundle of references a pane needs at input-handling time.
///
/// Phase-7 placeholder; populated in Phase 8/9.
#[allow(
    dead_code,
    reason = "Phase 7 placeholder ctx; populated pane-by-pane in Phase 8"
)]
pub struct PaneInputCtx<'a> {
    pub _phantom: std::marker::PhantomData<&'a ()>,
}

/// Bundle of references a pane reads to answer `is_navigable`.
///
/// Phase-7 placeholder; populated in Phase 8 when `is_navigable`
/// becomes load-bearing on the trait. Today's `is_pane_tabbable`
/// in `app/focus.rs` continues to drive tab order.
#[allow(
    dead_code,
    reason = "Phase 7 placeholder ctx; populated pane-by-pane in Phase 8"
)]
pub struct PaneNavCtx<'a> {
    pub _phantom: std::marker::PhantomData<&'a ()>,
}

/// Common behavior every pane provides.
///
/// Phase 7 declares the trait and lands `PaneId`-pure metadata
/// methods. Render, input, viewport access, and `is_navigable`
/// land in Phase 8 as each pane absorbs its state and body.
///
/// Phase 10 adds `fn hit_test(&self, row: u16) -> Option<HoverTarget>`.
#[allow(
    dead_code,
    reason = "Phase 7 foundation; first dispatch site wires up in Phase 8"
)]
pub trait Pane {
    // ── identity (Phase 7) ──────────────────────────────────────
    fn id(&self) -> PaneId;

    // ── routing metadata (Phase 7) ──────────────────────────────
    fn input_context(&self) -> InputContextKind;
    fn has_row_hitboxes(&self) -> bool;
    fn size_spec(&self, cpu_width: u16) -> PaneSizeSpec;

    // ── behavior (bodies migrate in Phases 8–9) ─────────────────
    //
    // Defaults panic with `unimplemented!()` so Phase 7 callers
    // who don't dispatch through the trait keep working. Phase 8
    // overrides per-pane as bodies migrate; Phase 9 finishes the
    // remaining seven panes; Phase 10 removes the panic and the
    // sink.
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: PaneRenderCtx<'_, '_>) {
        unimplemented!("render lands per-pane in Phase 8/9")
    }
    fn handle_input(&mut self, _event: &KeyEvent, _ctx: PaneInputCtx<'_>) {
        unimplemented!("handle_input lands per-pane in Phase 8/9")
    }
    fn is_navigable(&self, _ctx: PaneNavCtx<'_>) -> bool { false }
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

    #[test]
    fn input_context_kind_value_typed() {
        assert_eq!(InputContextKind::ProjectList, InputContextKind::ProjectList);
        assert_ne!(
            InputContextKind::ProjectList,
            InputContextKind::DetailFields
        );
    }
}

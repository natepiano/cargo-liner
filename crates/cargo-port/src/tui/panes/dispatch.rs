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
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneSizeSpec;

/// Read-only side of an in-flight render's hitbox-registration sink.
///
/// Phase 7: a placeholder type — skeleton impls don't render, so
/// no hitboxes flow through it yet. Phase 8/9 re-route the
/// `register_*_row_hitboxes` writes here. Phase 10 deletes the
/// type entirely when `Pane::hit_test` becomes a query method.
#[derive(Debug, Default)]
pub struct HitboxSink {
    _placeholder: (),
}

impl HitboxSink {
    /// A sink that discards every write. Used by Phase 7's
    /// (currently empty) flow and by isolated render tests.
    #[allow(
        dead_code,
        reason = "Phase 7 placeholder; first non-test caller wires up in Phase 8"
    )]
    pub const fn null() -> Self { Self { _placeholder: () } }
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

/// Bundle of references a pane needs at render time.
///
/// Phase-7 placeholder: fields populate pane-by-pane in Phase 8
/// as each pane's render body migrates and declares which
/// subsystems it reads. Listing all subsystem refs up front would
/// force every Phase-7 dispatch site to construct refs no pane
/// uses yet; we add them as bodies move.
#[allow(
    dead_code,
    reason = "Phase 7 placeholder ctx; populated pane-by-pane in Phase 8"
)]
pub struct PaneRenderCtx<'a> {
    pub focused_pane:      PaneId,
    pub focus_state:       PaneFocusState,
    pub animation_elapsed: std::time::Duration,
    pub hit_sink:          &'a mut HitboxSink,
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
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: PaneRenderCtx<'_>) {
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
    fn hitbox_sink_null_constructs() { let _sink = HitboxSink::null(); }

    #[test]
    fn input_context_kind_value_typed() {
        assert_eq!(InputContextKind::ProjectList, InputContextKind::ProjectList);
        assert_ne!(
            InputContextKind::ProjectList,
            InputContextKind::DetailFields
        );
    }
}

//! Higher-level input dispatch: the framework orchestrates the full
//! hit-test ladder so the embedding app contributes only the parts
//! that are genuinely app-domain (the app-owned modal overlay, the
//! tiled-pane registry, and three mapper hooks).

use ratatui::layout::Position;

use super::HitTestRegistry;
use super::hit_test_at;
use crate::FrameworkOverlayId;
use crate::ToastHit;

/// Outcome of [`Framework::hit_test_at`](crate::Framework::hit_test_at).
///
/// `Some(FrameworkHit)` means the framework participated in dispatch
/// for this click. The embedding app maps this into an optional
/// app-side target via [`InputContext::map_framework_hit`] — `None`
/// means "framework consumed the click but produced no actionable
/// target" (e.g. a framework modal overlay is open and the click
/// missed every selectable row, so the click is absorbed rather than
/// falling through to tiled panes below).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameworkHit {
    /// A toast hit (close button or card body).
    Toast(ToastHit),
    /// A framework overlay hit on `row` of overlay `id`.
    Overlay {
        /// Which framework overlay was hit.
        id:  FrameworkOverlayId,
        /// Selectable row index inside the overlay.
        row: usize,
    },
    /// A framework modal overlay is open but the click landed outside
    /// every selectable row. The framework swallows the click; no
    /// fall-through to lower layers.
    ModalMissed,
}

/// Hooks the framework needs from the embedding app to run the full
/// hit-test ladder.
///
/// `InputContext` extends [`HitTestRegistry`] (the tiled-pane walk).
/// The app supplies three additional pieces:
///
/// - Access to the framework so the orchestrator can run the toast + framework-overlay pass.
/// - An optional app-owned modal overlay hit-test (e.g. a finder popup). When `Some(target)` is
///   returned, dispatch returns it without walking the tiled panes.
/// - A mapper from [`FrameworkHit`] into the app's target type.
///
/// The framework owns the *order* of dispatch (toast → framework
/// overlay → app modal → tiled) so app code never re-derives it.
pub trait InputContext: HitTestRegistry {
    /// Framework-owned ladder: toasts and any open framework
    /// overlay. Implementations forward to
    /// [`Framework::hit_test_at`](crate::Framework::hit_test_at).
    /// `Some(_)` means the framework participated (and dispatch
    /// stops); `None` means fall through.
    fn framework_hit(&self, pos: Position) -> Option<FrameworkHit>;

    /// App-owned modal overlay hit-test. Outer `Some` means the
    /// app modal layer was open and claims the click — the inner
    /// `Option<Target>` is the row hit (or `None` if the click
    /// missed every row inside the overlay). Outer `None` means
    /// no app modal is open; dispatch falls through to the tiled
    /// walk.
    fn app_modal_overlay_hit(&self, pos: Position) -> Option<Option<Self::Target>>;

    /// Map a framework-side hit into the app's target type, or
    /// `None` when the hit was absorbed without producing an
    /// actionable row (see [`FrameworkHit::ModalMissed`]).
    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<Self::Target>;
}

/// Full hit-test dispatch.
///
/// Walks the framework-owned ladder first (toasts → framework
/// overlay → modal-miss block), then the app-owned modal overlay
/// short-circuit, then the tiled-pane z-order via
/// [`hit_test_at`](super::hit_test_at).
pub fn dispatch_hit_test<C: InputContext>(ctx: &C, pos: Position) -> Option<C::Target> {
    if let Some(hit) = ctx.framework_hit(pos) {
        return ctx.map_framework_hit(hit);
    }
    if let Some(target) = ctx.app_modal_overlay_hit(pos) {
        return target;
    }
    hit_test_at(ctx, pos)
}

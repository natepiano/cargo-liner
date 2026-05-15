//! Per-pane hit-test routing: the [`Hittable`] trait that panes
//! implement and the [`hit_test_at`] loop that walks a
//! [`HitTestRegistry`].

use ratatui::layout::Position;

use crate::Viewport;

/// Trait implemented by panes that participate in click / hover
/// dispatch.
///
/// `Target` is the concrete hit-result type the embedding application
/// uses (typically an enum carrying the matched pane id plus a row
/// index or affordance variant). The trait stays free of generic
/// associated types and supertraits so it is object-safe — the
/// dispatch loop in [`hit_test_at`] needs `&dyn Hittable<Target>`
/// to work. `Target` is a generic parameter rather than an associated
/// type so impls for foreign types (the framework's own pane structs)
/// can be written in the embedding crate against an
/// embedding-defined `Target` without tripping the orphan rule.
pub trait Hittable<Target> {
    /// Return the hit target if `pos` lands inside this pane's
    /// rendered area, or `None` otherwise.
    fn hit_test_at(&self, pos: Position) -> Option<Target>;
}

/// Top-down walk of every hittable pane in stacking order.
///
/// The embedding application implements this on whatever struct
/// already owns refs to every pane (often the top-level `App` or a
/// dedicated `Panes` aggregate). `z_order` returns the static
/// ordering; `pane` maps a pane id to the trait object that answers
/// hit queries. [`hit_test_at`] walks the order, returning the first
/// non-`None` hit.
pub trait HitTestRegistry {
    /// Pane identifier carried in the z-order array.
    type PaneId: Copy + 'static;
    /// Concrete hit-result type produced by every pane in this
    /// registry.
    type Target;
    /// Top-of-stack-first ordering of every hittable pane id.
    fn z_order() -> &'static [Self::PaneId];
    /// Borrow the pane for `id` as a trait object, or `None` if the
    /// id is not currently realized.
    fn pane(&self, id: Self::PaneId) -> Option<&dyn Hittable<Self::Target>>;

    /// Mutably borrow the [`Viewport`] for `id`, or `None` if the id
    /// has no hover-tracking viewport. Used by framework helpers
    /// such as [`clear_all_hover`] to mutate per-pane viewport state
    /// without the embedding app re-deriving the per-pane match at
    /// every call site.
    fn viewport_mut(&mut self, id: Self::PaneId) -> Option<&mut Viewport>;
}

/// Clear hovered-row state on every tiled pane in `registry`.
///
/// Walks `registry`'s z-order and calls `set_hovered(None)` on each
/// pane's [`Viewport`]. Framework-owned panes (toasts, framework
/// overlays) are outside this registry; clear them separately via
/// [`Framework::clear_hover`](crate::Framework::clear_hover).
pub fn clear_all_hover<R: HitTestRegistry>(registry: &mut R) {
    for id in R::z_order() {
        if let Some(v) = registry.viewport_mut(*id) {
            v.set_hovered(None);
        }
    }
}

/// First-hit-wins dispatch: walk `registry.z_order()` top-down and
/// return the first pane's hit, or `None` if no pane claims `pos`.
pub fn hit_test_at<R: HitTestRegistry>(registry: &R, pos: Position) -> Option<R::Target> {
    for id in R::z_order() {
        if let Some(pane) = registry.pane(*id)
            && let Some(hit) = pane.hit_test_at(pos)
        {
            return Some(hit);
        }
    }
    None
}

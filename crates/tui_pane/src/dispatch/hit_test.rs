//! Generic hit-test dispatch loop.

use ratatui::layout::Position;

use super::Hittable;
use crate::Viewport;

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
    /// such as [`clear_all_hover`] and [`set_pane_pos`] to mutate
    /// per-pane viewport state without the embedding app re-deriving
    /// the per-pane match at every call site.
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

/// Set the cursor row on the tiled pane identified by `id`. No-op
/// when the id has no hover-tracking viewport.
pub fn set_pane_pos<R: HitTestRegistry>(registry: &mut R, id: R::PaneId, row: usize) {
    if let Some(v) = registry.viewport_mut(id) {
        v.set_pos(row);
    }
}

/// Set the hovered row on the tiled pane identified by `id`. No-op
/// when the id has no hover-tracking viewport.
pub fn set_pane_hovered<R: HitTestRegistry>(registry: &mut R, id: R::PaneId, row: Option<usize>) {
    if let Some(v) = registry.viewport_mut(id) {
        v.set_hovered(row);
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

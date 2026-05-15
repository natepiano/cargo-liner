//! Generic render dispatch: the [`Renderable`] trait, the
//! [`PaneRegistry`] mapping from pane id to render target, and the
//! [`render_panes`] loop that ties them together.
//!
//! Symmetric with [`super::Hittable`] / [`super::HitTestRegistry`] /
//! [`super::hit_test_at`] on the input side. Each pane implements
//! [`Renderable`] against its embedding application's render-context
//! type; the embedding crate hands out `&mut dyn Renderable` trait
//! objects from a [`PaneRegistry`]; [`render_panes`] walks the resolved
//! layout and dispatches each one.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ResolvedPaneLayout;

/// Per-pane render dispatch.
///
/// `Ctx` is the embedding application's render-context type — a
/// bundle of references each pane reads at render time. Cargo-port
/// instantiates this with its `PaneRenderCtx<'_>`; other embeddings
/// can choose their own context type. `Ctx` is a generic parameter
/// rather than an associated type so impls for foreign types (the
/// framework's own pane structs) can be written in the embedding
/// crate against an embedding-defined context without tripping the
/// orphan rule — same reasoning as [`super::Hittable`].
pub trait Renderable<Ctx> {
    /// Draw the pane into `area` of `frame`, reading `ctx` for the
    /// refs the pane needs.
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx);
}

/// Pane-id-keyed mapping from layout entry to render target.
///
/// The embedding crate implements this on whatever struct already
/// holds disjoint `&mut` references to every renderable pane. The
/// associated [`Self::Ctx`] is a generic-associated lifetime so the
/// same registry can be driven by render contexts whose borrows
/// outlive (or come from a different scope than) the registry itself
/// — the higher-ranked trait bound in [`Self::pane_mut`]'s return
/// type spells this out.
pub trait PaneRegistry {
    /// Pane identifier carried in the resolved layout.
    type PaneId: Copy;
    /// Render context produced by the embedding crate. The
    /// generic-associated lifetime lets each call to
    /// [`render_panes`] supply a fresh borrow scope.
    type Ctx<'a>;
    /// Borrow the pane registered under `id` as a render trait
    /// object, or `None` when the id is not currently realized.
    ///
    /// The higher-ranked trait bound (`for<'a>`) says the returned
    /// pane can render against any lifetime of `Self::Ctx`; impls
    /// usually satisfy this via `impl<'a> Renderable<Ctx<'a>> for X`
    /// (or the elided sugar `impl Renderable<Ctx<'_>> for X`).
    fn pane_mut(&mut self, id: Self::PaneId) -> Option<&mut dyn for<'a> Renderable<Self::Ctx<'a>>>;
}

/// Walk `layout` in resolved order, asking `registry` for each
/// pane's render trait object and dispatching it against `ctx`.
///
/// This is the framework-side replacement for the embedding crate's
/// per-pane match in its top-level render fn. Panes whose id is
/// absent from the registry are skipped silently.
pub fn render_panes<R: PaneRegistry>(
    frame: &mut Frame<'_>,
    registry: &mut R,
    layout: &ResolvedPaneLayout<R::PaneId>,
    ctx: &R::Ctx<'_>,
) {
    for resolved in &layout.panes {
        if let Some(pane) = registry.pane_mut(resolved.pane) {
            pane.render(frame, resolved.area, ctx);
        }
    }
}

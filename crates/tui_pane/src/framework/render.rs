//! Generic render dispatch: the [`Renderable`] trait, the
//! [`PaneRegistry`] mapping from pane id to render target, and the
//! [`render_panes`] loop that ties them together.
//!
//! Symmetric with [`Hittable`](super::hit_test::Hittable) /
//! [`HitTestRegistry`](super::hit_test::HitTestRegistry) /
//! [`hit_test_at`](super::hit_test::hit_test_at) on the input side.
//! Each pane implements
//! [`Renderable`] against its embedding application's render-context
//! type; the embedding crate hands out `&mut dyn Renderable` trait
//! objects from a [`PaneRegistry`]; [`render_panes`] walks the resolved
//! layout and dispatches each one.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::GridLines;
use crate::PaneBorders;
use crate::PaneChrome;
use crate::PaneFrame;
use crate::PaneFrameLabel;
use crate::ResolvedPaneLayout;

/// What the shared frame draws around one pane.
///
/// A pane hands this back rather than drawing its own [`Block`]: the
/// border it would draw is the same line its neighbour would draw, and
/// the title and affordances it would write sit on that line. One pass
/// owns all of it -- see [`GridLines`].
///
/// [`Block`]: ratatui::widgets::Block
#[derive(Clone, Debug, Default)]
pub struct PaneFrameChrome {
    /// Written over the pane's top border line.
    pub title:   String,
    /// Whether the pane holds focus, which decides the shade of every
    /// line it touches and whether its contents sit on the focus tint.
    pub focused: bool,
    /// The pane's own interior rules, each reaching from one border line
    /// to the other so the pass works out where they cross.
    pub rules:   Vec<Rect>,
    /// Text to write over the finished lines -- a scroll affordance on
    /// the bottom border, and anything like it.
    pub labels:  Vec<PaneFrameLabel>,
}

/// Per-pane render dispatch.
///
/// `Ctx` is the embedding application's render-context type — a
/// bundle of references each pane reads at render time. Cargo-port
/// instantiates this with its `PaneRenderCtx<'_>`; other embeddings
/// can choose their own context type. `Ctx` is a generic parameter
/// rather than an associated type so impls for foreign types (the
/// framework's own pane structs) can be written in the embedding
/// crate against an embedding-defined context without tripping the
/// orphan rule — same reasoning as [`Hittable`](super::hit_test::Hittable).
pub trait Renderable<Ctx> {
    /// Draw the pane into `area` of `frame`, reading `ctx` for the
    /// refs the pane needs.
    ///
    /// Answers with the frame it wants drawn around it, or `None` when
    /// it draws its own chrome -- which is what an overlay or a popup
    /// does. Handing the chrome back at the end of the render rather
    /// than declaring it up front is what lets a title report what the
    /// pane just laid out: a count, a cursor position, a follow state
    /// off a viewport it only syncs while rendering.
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx) -> Option<PaneFrameChrome>;
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
    chrome: PaneChrome,
    borders: PaneBorders,
) {
    let bounds = layout.bounds();
    let mut grid_lines = GridLines::new(bounds);
    for resolved in &layout.panes {
        // Under `Shared`, each pane reaches one line onto the neighbours
        // below and to the right of it, so the boundary between the two
        // of them is a single line they share rather than two lines side
        // by side. Under `Separate` the pane keeps the rect the layout
        // resolved, and its neighbour's line sits beside its own.
        let area = borders.pane_area(resolved.area, bounds);
        let Some(pane) = registry.pane_mut(resolved.pane) else {
            continue;
        };
        let Some(pane_chrome) = pane.render(frame, area, ctx) else {
            continue;
        };
        let pane_frame = PaneFrame::new(area).with_focus(pane_chrome.focused);
        grid_lines.add_titled(pane_frame, pane_chrome.title);
        for rule in pane_chrome.rules {
            grid_lines.add_rule(pane_frame, rule);
        }
        for label in pane_chrome.labels {
            grid_lines.add_label(pane_frame, label);
        }
    }
    grid_lines.render(frame.buffer_mut(), chrome, borders);
}

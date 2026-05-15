//! Generic dispatch primitives for routing input and render to panes.

mod hit_test;
mod input;
mod render;

pub use hit_test::HitTestRegistry;
pub use hit_test::clear_all_hover;
pub use hit_test::hit_test_at;
pub use hit_test::set_pane_hovered;
pub use hit_test::set_pane_pos;
pub use input::FrameworkHit;
pub use input::InputContext;
pub use input::dispatch_hit_test;
use ratatui::layout::Position;
pub use render::PaneRegistry;
pub use render::Renderable;
pub use render::render_panes;

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

//! The `Pane` trait, the `Hittable` sub-trait, and the
//! `PaneRenderCtx` bundle.
//!
//! Each clickable pane retains its own hit-test layout (computed
//! during render) and answers `Hittable::hit_test_at(pos)`
//! directly, rather than pushing hitboxes into a global vec.
//!
//! Lives at `crate::tui::pane::dispatch` (not `crate::tui::panes::dispatch`)
//! so `pub(super)` reaches `crate::tui` — every subsystem under `tui/`
//! can `impl Pane` without widening the trait's visibility.

use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use strum::EnumIter;
use tui_pane::ToastId;

use super::DismissTarget;
use super::PaneFocusState;
use crate::tui::panes::PaneId;
use crate::tui::project_list::ProjectList;
use crate::tui::state::Config;

/// Bundle of references a pane needs at render time.
pub struct PaneRenderCtx<'a> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     Duration,
    pub config:                &'a Config,
    pub project_list:          &'a ProjectList,
    pub selected_project_path: Option<&'a Path>,
}

/// Per-pane render dispatch. `Hittable` is a separate sub-trait
/// for panes that participate in click/hover dispatch.
pub trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>);
}

/// Result of a single pane's hit-test at a screen position.
#[derive(Clone, Debug)]
pub enum HoverTarget {
    PaneRow { pane: PaneId, row: usize },
    Dismiss(DismissTarget),
    ToastCard(ToastId),
}

/// Sub-trait implemented only by panes that participate in click /
/// hover dispatch. Keeping `Pane` and `Hittable` separate lets the
/// dispatch match in `Panes::hit_test_at` reject non-clickable
/// panes at compile time.
pub trait Hittable: Pane {
    /// Return the hit target if `pos` lands inside this pane's
    /// rendered area, or `None` otherwise. Implementations rely on
    /// state recorded during render (viewport content area + scroll
    /// offset for uniform-row panes; per-row rect lists for non-
    /// uniform panes; per-toast rects for the toasts overlay).
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget>;
}

/// Compile-time enumeration of every pane that implements
/// `Hittable`. The `strum::EnumIter` derive lets the unit test in
/// `hit_test_tests` walk all variants and assert each one appears
/// in `HITTABLE_Z_ORDER`.
#[derive(EnumIter, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HittableId {
    Toasts,
    Finder,
    Settings,
    Keymap,
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
}

/// Stacking order used by `Panes::hit_test_at`: top of stack first.
/// Overlays sit above tiled panes; within a category the order
/// matches how panes are drawn (later-drawn overlays occlude earlier
/// ones on the screen).
pub const HITTABLE_Z_ORDER: [HittableId; 12] = [
    HittableId::Toasts,
    HittableId::Finder,
    HittableId::Settings,
    HittableId::Keymap,
    HittableId::ProjectList,
    HittableId::Package,
    HittableId::Lang,
    HittableId::Cpu,
    HittableId::Git,
    HittableId::Targets,
    HittableId::Lints,
    HittableId::CiRuns,
];

#[cfg(test)]
mod hit_test_tests {
    use std::collections::HashSet;

    use strum::IntoEnumIterator;

    use super::HITTABLE_Z_ORDER;
    use super::HittableId;

    #[test]
    fn z_order_covers_every_hittable_id() {
        let in_order: HashSet<HittableId> = HITTABLE_Z_ORDER.iter().copied().collect();
        let all: HashSet<HittableId> = HittableId::iter().collect();
        assert_eq!(
            in_order, all,
            "every HittableId must appear exactly once in HITTABLE_Z_ORDER"
        );
        assert_eq!(
            HITTABLE_Z_ORDER.len(),
            in_order.len(),
            "HITTABLE_Z_ORDER must not contain duplicates"
        );
    }
}

//! The `Pane` trait, the `PaneRenderCtx` bundle, and the `HittableId`
//! z-order discriminant.
//!
//! Each clickable pane retains its own hit-test layout (computed
//! during render) and answers
//! [`Hittable::hit_test_at`](tui_pane::Hittable::hit_test_at) directly,
//! rather than pushing hitboxes into a global vec.
//!
//! Lives at `crate::tui::pane::dispatch` (not `crate::tui::panes::dispatch`)
//! so `pub(super)` reaches `crate::tui` — every subsystem under `tui/`
//! can `impl Pane` without widening the trait's visibility.

use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
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

/// Per-pane render dispatch.
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

/// Compile-time enumeration of every tiled pane that implements
/// [`tui_pane::Hittable`]. The `strum::EnumIter` derive lets the unit
/// test in `hit_test_tests` walk all variants and assert each one
/// appears in `HITTABLE_Z_ORDER`.
///
/// Toasts and the framework overlays (Keymap, Settings) are dispatched
/// by [`tui_pane::dispatch_hit_test`] through the framework's own
/// hit-test ladder, not through this registry. The app-modal Finder
/// overlay is dispatched via
/// [`tui_pane::InputContext::app_modal_overlay_hit`]. None of those
/// three appear here.
#[derive(EnumIter, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HittableId {
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
}

/// Stacking order used for tiled-pane hit-test dispatch: top of stack
/// first. Overlays and toasts are not here — see [`HittableId`].
pub const HITTABLE_Z_ORDER: [HittableId; 8] = [
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

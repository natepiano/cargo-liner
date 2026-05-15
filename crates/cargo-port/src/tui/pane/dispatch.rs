//! The `PaneRenderCtx` bundle, the `HoverTarget` hit-result, and the
//! `HittableId` z-order discriminant for the cargo-port hit-test
//! registry.
//!
//! Each clickable pane retains its own hit-test layout (computed
//! during render) and answers
//! [`Hittable::hit_test_at`](tui_pane::Hittable::hit_test_at) directly,
//! rather than pushing hitboxes into a global vec. Render dispatch
//! goes through [`tui_pane::Renderable`] — impls live alongside each
//! pane struct.

use std::path::Path;
use std::time::Duration;

use strum::EnumIter;
use tui_pane::ToastId;

use super::DismissTarget;
use crate::tui::keymap_ui::KeymapRenderInputs;
use crate::tui::panes::PaneId;
use crate::tui::project_list::ProjectList;
use crate::tui::settings::SettingsRenderInputs;
use crate::tui::state::CiStatusLookup;
use crate::tui::state::Config;
use crate::tui::state::Inflight;
use crate::tui::state::Scan;

/// Bundle of references a pane needs at render time.
///
/// Every field is uniform across the tile-render pass: every pane in
/// the loop reads the same context. Per-pane state lives on the
/// pane structs themselves (focus snapshot via
/// [`tui_pane::RenderFocus`], precomputed CI status cache on
/// `ProjectListPane`), set by App immediately before
/// [`tui_pane::render_panes`] runs. That separation is what lets the
/// generic dispatch loop carry one `&PaneRenderCtx` for the entire
/// frame.
pub(crate) struct PaneRenderCtx<'a> {
    pub(crate) animation_elapsed:      Duration,
    pub(crate) config:                 &'a Config,
    pub(crate) project_list:           &'a ProjectList,
    pub(crate) selected_project_path:  Option<&'a Path>,
    /// In-flight runtime state read by tiled panes during render
    /// (currently only `OutputPane` for the running-example title
    /// and the captured output lines).
    pub(crate) inflight:               &'a Inflight,
    /// Scan subsystem ref. Needed by `ProjectListPane::render` for
    /// discovery-shimmer lookups; tiled detail panes leave it
    /// unread.
    pub(crate) scan:                   &'a Scan,
    /// Pre-render CI snapshot built from `&Ci` before the dispatch
    /// loop runs. `ProjectListPane` reads CI status per row through
    /// this lookup instead of holding `&Ci` directly, which lets
    /// the CI pane's own dispatcher consume `&mut self.ci` in the
    /// same pass.
    pub(crate) ci_status_lookup:       &'a CiStatusLookup,
    /// Precomputed render inputs for the Keymap overlay. `None` for
    /// every render path that isn't the Keymap overlay dispatcher;
    /// `Some` when the overlay is open and `KeymapPane`'s
    /// [`tui_pane::Renderable`] impl is about to draw the popup.
    /// Built by [`crate::tui::keymap_ui::prepare_keymap_render_inputs`]
    /// before `App::split_for_render`, so the still-current `&App`
    /// borrow can walk `framework_keymap`.
    pub(crate) keymap_render_inputs:   Option<&'a KeymapRenderInputs>,
    /// Precomputed render inputs for the Settings overlay. `None`
    /// for every render path that isn't the Settings overlay
    /// dispatcher; `Some` when the overlay is open. Built by
    /// [`crate::tui::settings::prepare_settings_render_inputs`].
    pub(crate) settings_render_inputs: Option<&'a SettingsRenderInputs>,
    /// Inline error string from the overlays subsystem (Settings /
    /// Keymap inline-error line). `None` when no error is pinned.
    /// Reserved for the deferred Keymap / Settings overlay
    /// absorption — populated today, consumed once those panes
    /// gain real `Renderable::render` bodies.
    #[allow(
        dead_code,
        reason = "reserved for Keymap / Settings overlay absorption — populated today, \
                  consumed once those panes gain real Renderable::render bodies"
    )]
    pub(crate) inline_error:           Option<&'a str>,
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

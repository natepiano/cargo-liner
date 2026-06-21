use tui_pane::SettingsPane;

use super::overlays::FinderPane;
use super::panes::CpuPane;
use super::panes::GitPane;
use super::panes::LangPane;
use super::panes::OutputPane;
use super::panes::PackagePane;
use super::panes::ProjectListPane;
use super::panes::TargetsPane;
use super::project_list::ProjectList;
use super::render_context::PaneRenderCtx;
use super::running_targets::RunningTargets;
use super::settings::SettingsRenderInputs;
use super::state::Ci;
use super::state::Config;
use super::state::Inflight;
use super::state::Lint;
use super::state::Scan;

/// Render-time borrows of `App`. Holds disjoint `&mut` references to
/// every renderable pane (organized for [`tui_pane::PaneRegistry`])
/// alongside a fully-built [`PaneRenderCtx`] whose borrows are
/// disjoint from the registry's. Single source of truth for "what
/// each frame's render loop needs."
pub(super) struct RenderBorrows<'a> {
    pub(super) registry:        RenderRegistry<'a>,
    pub(super) pane_render_ctx: PaneRenderCtx<'a>,
}

/// Optional precomputed render inputs for framework overlays.
#[derive(Clone, Copy)]
pub(super) struct OverlayRenderInputs<'a> {
    pub(super) settings: Option<&'a SettingsRenderInputs>,
}

impl<'a> OverlayRenderInputs<'a> {
    pub const fn none() -> Self { Self { settings: None } }

    pub const fn settings(inputs: &'a SettingsRenderInputs) -> Self {
        Self {
            settings: Some(inputs),
        }
    }
}

/// Disjoint `&mut` borrows of every renderable pane on `App`.
/// Cargo-port's [`tui_pane::PaneRegistry`] impl lives on this type;
/// the embedding-side match in [`tui_pane::render_panes`]'s loop
/// hands out each entry as a `&mut dyn Renderable`.
pub(super) struct RenderRegistry<'a> {
    pub(super) package:       &'a mut PackagePane,
    pub(super) lang:          &'a mut LangPane,
    pub(super) cpu:           &'a mut CpuPane,
    pub(super) git:           &'a mut GitPane,
    pub(super) targets:       &'a mut TargetsPane,
    pub(super) project_list:  &'a mut ProjectListPane,
    pub(super) output:        &'a mut OutputPane,
    pub(super) lint:          &'a mut Lint,
    pub(super) ci:            &'a mut Ci,
    pub(super) settings_pane: &'a mut SettingsPane,
}

/// Result of `App::split_finder_for_render`.
pub(super) struct FinderSplit<'a> {
    pub(super) finder_pane:     &'a mut FinderPane,
    pub(super) config:          &'a Config,
    pub(super) project_list:    &'a ProjectList,
    pub(super) inflight:        &'a Inflight,
    pub(super) scan:            &'a Scan,
    pub(super) running_targets: &'a RunningTargets,
}

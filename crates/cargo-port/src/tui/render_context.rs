use std::path::Path;
use std::time::Duration;

use super::panes::SyncedDescriptionHeight;
use super::project_list::ProjectList;
use super::running_targets::RunningTargets;
use super::settings::SettingsRenderInputs;
use super::state::CiStatusLookup;
use super::state::Config;
use super::state::Inflight;
use super::state::Scan;

/// Bundle of references a pane needs at render time.
///
/// Every field is uniform across the tile-render pass: every pane in
/// the loop reads the same context. Per-pane state lives on the pane structs
/// themselves, set by `App` immediately before `tui_pane::render_panes` runs.
pub(super) struct PaneRenderCtx<'a> {
    pub(super) animation_elapsed:         Duration,
    pub(super) config:                    &'a Config,
    pub(super) project_list:              &'a ProjectList,
    pub(super) selected_project_path:     Option<&'a Path>,
    pub(super) inflight:                  &'a Inflight,
    pub(super) scan:                      &'a Scan,
    pub(super) ci_status_lookup:          &'a CiStatusLookup,
    pub(super) settings_render_inputs:    Option<&'a SettingsRenderInputs>,
    pub(super) synced_description_height: SyncedDescriptionHeight,
    pub(super) running_targets:           &'a RunningTargets,
}

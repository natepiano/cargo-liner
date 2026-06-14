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
pub(crate) struct PaneRenderCtx<'a> {
    pub(crate) animation_elapsed:         Duration,
    pub(crate) config:                    &'a Config,
    pub(crate) project_list:              &'a ProjectList,
    pub(crate) selected_project_path:     Option<&'a Path>,
    pub(crate) inflight:                  &'a Inflight,
    pub(crate) scan:                      &'a Scan,
    pub(crate) ci_status_lookup:          &'a CiStatusLookup,
    pub(crate) settings_render_inputs:    Option<&'a SettingsRenderInputs>,
    pub(crate) synced_description_height: SyncedDescriptionHeight,
    pub(crate) running_targets:           &'a RunningTargets,
}

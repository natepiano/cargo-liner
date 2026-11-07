use std::path::Path;
use std::time::Duration;

use tui_pane::ColorMode;

use super::panes::EmptyDescriptionBehavior;
use super::panes::OutputPresentation;
use super::panes::ProjectPresentation;
use super::panes::SyncedDescriptionHeight;
use super::project_list_state::ProjectList;
use super::settings::SettingsRenderInputs;
use super::state::CiStatusLookup;
use super::state::Config;
use super::state::Scan;

/// Bundle of references a pane needs at render time.
///
/// Every field is uniform across the tile-render pass: every pane in
/// the loop reads the same context. Per-pane state lives on the pane structs
/// themselves, set by `App` immediately before `tui_pane::render_panes` runs.
pub(super) struct PaneRenderCtx<'a> {
    pub(super) color_mode:                ColorMode,
    pub(super) animation_elapsed:         Duration,
    pub(super) config:                    &'a Config,
    pub(super) project_list:              &'a ProjectList,
    pub(super) selected_project_path:     Option<&'a Path>,
    pub(super) scan:                      &'a Scan,
    pub(super) ci_status_lookup:          &'a CiStatusLookup,
    pub(super) settings_render_inputs:    Option<&'a SettingsRenderInputs>,
    pub(super) synced_description_height: SyncedDescriptionHeight,
    pub(super) empty_package_description: EmptyDescriptionBehavior,
    /// The one value the Output pane's layout, visibility, focus, labels,
    /// copy availability, hit testing, and rendering all derive from.
    pub(super) output_presentation:       OutputPresentation<'a>,
}

impl PaneRenderCtx<'_> {
    pub(super) fn project_presentation(&self) -> ProjectPresentation {
        if self
            .selected_project_path
            .is_some_and(|path| self.project_list.is_deleted(path))
        {
            ProjectPresentation::Deleted
        } else {
            ProjectPresentation::Current
        }
    }
}

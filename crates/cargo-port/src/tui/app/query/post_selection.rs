use crate::project::ProjectCiData;
use crate::tui::app::App;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitRow;
use crate::tui::panes::PaneId;
use crate::tui::shortcuts::InputContext;

impl App {
    /// Returns the Enter-key action label for the current cursor position,
    /// or `None` if Enter does nothing here. Used by the shortcut bar to
    /// only show Enter when it's actionable.
    pub fn enter_action(&self) -> Option<&'static str> {
        match self.input_context() {
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.focus.base() == PaneId::Package {
                    let pkg = self.panes.package.content()?;
                    let fields = panes::package_fields_from_data(pkg);
                    let field = *fields.get(self.panes.package.viewport.pos())?;
                    if field == DetailField::CratesIo && pkg.crates_version.is_some() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    let git = self.panes.git.content()?;
                    let pos = self.panes.git.viewport.pos();
                    match panes::git_row_at(git, pos) {
                        Some(GitRow::Remote(remote)) if remote.full_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_info = self
                    .project_list
                    .selected_project_path()
                    .and_then(|path| self.project_list.ci_info_for(path));
                let run_count = ci_info.map_or(0, |info| info.runs.len());
                let selected_path = self.project_list.selected_project_path();
                if self.ci.viewport.pos() == run_count
                    && !selected_path.is_some_and(|path| self.ci.fetch_tracker.is_fetching(path))
                    && !selected_path.is_some_and(|path| {
                        self.project_list
                            .ci_data_for(path)
                            .is_some_and(ProjectCiData::is_exhausted)
                    })
                {
                    Some("fetch")
                } else {
                    None
                }
            },
            _ => None,
        }
    }
}

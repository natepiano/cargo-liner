use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitRow;
use crate::tui::panes::PaneId;
use crate::tui::shortcuts::InputContext;

impl App {
    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project_path().map(AbsolutePath::from);
        if self
            .project_list
            .paths()
            .collapsed_anchor
            .as_ref()
            .is_some_and(|anchor| current.as_ref() != Some(anchor))
        {
            self.project_list.paths_mut().collapsed_selected = None;
            self.project_list.paths_mut().collapsed_anchor = None;
        }
        if self.project_list.paths_mut().selected_project == current {
            return;
        }

        self.project_list
            .paths_mut()
            .selected_project
            .clone_from(&current);
        self.reset_project_panes();

        let panes = self.tabbable_panes();
        if !panes.contains(&self.focus.base()) {
            self.focus.set(PaneId::ProjectList);
        }

        if self.focus.overlay_return().is_some() && !self.focus.overlay_return_is_in(&panes) {
            self.focus.retarget_overlay_return(PaneId::ProjectList);
        }

        if let Some(abs_path) = current
            && self.project_list.paths_mut().last_selected.as_ref() != Some(&abs_path)
        {
            self.scan.bump_generation();
            self.project_list.paths_mut().last_selected = Some(abs_path);
            self.project_list.mark_sync_changed();
            self.maybe_priority_fetch();
        }
    }

    /// Returns the Enter-key action label for the current cursor position,
    /// or `None` if Enter does nothing here. Used by the shortcut bar to
    /// only show Enter when it's actionable.
    pub fn enter_action(&self) -> Option<&'static str> {
        match self.input_context() {
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.focus.base() == PaneId::Package {
                    let pkg = self.panes.package().content()?;
                    let fields = panes::package_fields_from_data(pkg);
                    let field = *fields.get(self.panes.package().viewport().pos())?;
                    if field == DetailField::CratesIo && pkg.crates_version.is_some() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    let git = self.panes.git().content()?;
                    let pos = self.panes.git().viewport().pos();
                    match panes::git_row_at(git, pos) {
                        Some(GitRow::Remote(remote)) if remote.full_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_info = self
                    .selected_project_path()
                    .and_then(|path| self.projects().ci_info_for(path));
                let run_count = ci_info.map_or(0, |info| info.runs.len());
                let selected_path = self.selected_project_path();
                if self.ci.viewport().pos() == run_count
                    && !selected_path.is_some_and(|path| self.ci_is_fetching(path))
                    && !selected_path.is_some_and(|path| self.ci_is_exhausted(path))
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

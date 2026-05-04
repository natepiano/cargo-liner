use std::path::Path;

use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::RootItem;
use crate::tui::app::App;

impl App {
    pub fn selected_ci_path(&self) -> Option<AbsolutePath> {
        let path = self.selected_project_path()?;
        let entry = self.projects().entry_containing(path)?;
        Some(entry.item.path().clone())
    }

    pub fn selected_ci_runs(&self) -> Vec<CiRun> {
        self.selected_project_path()
            .map_or_else(Vec::new, |path| self.ci_runs_for_display(path))
    }

    pub fn unpublished_ci_branch_name(&self, path: &Path) -> Option<String> {
        let git = self.projects().git_info_for(path)?;
        let default_branch = self
            .projects()
            .repo_info_for(path)
            .and_then(|repo| repo.default_branch.as_deref());
        (git.primary_tracked_ref().is_none() && git.branch.as_deref() != default_branch)
            .then(|| git.branch.clone())
            .flatten()
    }

    pub fn ci_for(&self, path: &Path) -> Option<Conclusion> {
        // A branch with no upstream tracking can't have CI runs — don't
        // show the parent repo's result for an unpushed worktree branch.
        if self.unpublished_ci_branch_name(path).is_some() {
            return None;
        }
        self.ci_info_for(path)
            .and_then(|_| self.latest_ci_run_for_path(path))
            .map(|run| run.conclusion)
    }

    pub fn ci_data_for(&self, path: &Path) -> Option<&ProjectCiData> {
        self.projects()
            .entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref())
            .map(|repo| &repo.ci_data)
    }

    pub fn ci_info_for(&self, path: &Path) -> Option<&ProjectCiInfo> {
        self.ci_data_for(path).and_then(ProjectCiData::info)
    }

    pub fn ci_is_fetching(&self, path: &Path) -> bool {
        self.projects().entry_containing(path).is_some_and(|entry| {
            self.ci
                .fetch_tracker()
                .is_fetching(entry.item.path().as_path())
        })
    }

    pub(super) fn ci_is_exhausted(&self, path: &Path) -> bool {
        self.ci_data_for(path)
            .is_some_and(ProjectCiData::is_exhausted)
    }

    /// Aggregate CI for a `RootItem`.
    pub fn ci_for_item(&self, item: &RootItem) -> Option<Conclusion> {
        let paths = Self::unique_item_paths(item);
        if paths.len() == 1 {
            return self.ci_for(&paths[0]);
        }
        let mut any_red = false;
        let mut all_green = true;
        let mut any_data = false;
        for path in &paths {
            if let Some(run) = self.latest_ci_run_for_path(path) {
                any_data = true;
                if run.conclusion.is_failure() {
                    any_red = true;
                    all_green = false;
                } else if !run.conclusion.is_success() {
                    all_green = false;
                }
            }
        }
        if !any_data {
            None
        } else if any_red {
            Some(Conclusion::Failure)
        } else if all_green {
            Some(Conclusion::Success)
        } else {
            None
        }
    }
}

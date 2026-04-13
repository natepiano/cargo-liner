use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use super::App;
use super::types::CiRunDisplayMode;
use super::types::CiState;
use crate::ci;
use crate::ci::CiRun;
use crate::project::ProjectFields;
use crate::scan;
use crate::scan::CiFetchResult;
use crate::tui::detail::CiFetchKind;

impl App {
    pub(super) fn owner_repo_for_path_inner(&self, path: &Path) -> Option<ci::OwnerRepo> {
        let owner_path = self.ci_owner_path_for_inner(path)?;
        self.git_info_for(owner_path.as_path())
            .and_then(|git| git.url.as_deref())
            .and_then(ci::parse_owner_repo)
    }

    pub(super) fn owner_paths_for_repo_inner(&self, repo: &ci::OwnerRepo) -> Vec<PathBuf> {
        let mut owner_paths = Vec::new();
        self.projects.for_each_leaf_path(|path, _| {
            if !self.is_ci_owner_path(path) {
                return;
            }
            let Some(git) = self.git_info_for(path) else {
                return;
            };
            let Some(url) = git.url.as_deref() else {
                return;
            };
            if ci::parse_owner_repo(url).as_ref() == Some(repo) {
                owner_paths.push(path.to_path_buf());
            }
        });
        owner_paths
    }

    pub(super) fn ci_owner_path_for_inner(&self, path: &Path) -> Option<PathBuf> {
        for item in &self.projects {
            match item {
                crate::project::RootItem::Rust(crate::project::RustProject::Workspace(ws))
                    if path.starts_with(ws.path()) =>
                {
                    return Some(ws.path().to_path_buf());
                },
                crate::project::RootItem::Rust(crate::project::RustProject::Package(pkg))
                    if path.starts_with(pkg.path()) =>
                {
                    return Some(pkg.path().to_path_buf());
                },
                crate::project::RootItem::NonRust(project) if project.path() == path => {
                    return Some(project.path().to_path_buf());
                },
                crate::project::RootItem::Worktrees(
                    crate::project::WorktreeGroup::Workspaces {
                        primary, linked, ..
                    },
                ) => {
                    for ws in std::iter::once(primary).chain(linked.iter()) {
                        if path.starts_with(ws.path()) {
                            return Some(ws.path().to_path_buf());
                        }
                    }
                },
                crate::project::RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
                    primary,
                    linked,
                    ..
                }) => {
                    for pkg in std::iter::once(primary).chain(linked.iter()) {
                        if path.starts_with(pkg.path()) {
                            return Some(pkg.path().to_path_buf());
                        }
                    }
                },
                _ => {},
            }
        }
        None
    }

    /// Insert CI runs from the initial scan for a CI owner path.
    pub(super) fn insert_ci_runs(&mut self, path: &Path, runs: Vec<CiRun>, github_total: u32) {
        let abs = path.to_path_buf();
        if !self.is_cargo_active_path(&abs) {
            self.ci_state.remove(&abs);
            return;
        }
        let exhausted = self
            .git_info_for(&abs)
            .and_then(|git| {
                git.url.as_ref().and_then(|url| {
                    ci::parse_owner_repo(url)
                        .map(|owner_repo| scan::is_exhausted(owner_repo.owner(), owner_repo.repo()))
                })
            })
            .unwrap_or(false);
        self.ci_state.insert(
            abs,
            CiState::Loaded {
                runs,
                exhausted,
                github_total,
            },
        );
    }

    /// Process a completed CI fetch: merge runs and detect exhaustion.
    pub(super) fn handle_ci_fetch_complete(
        &mut self,
        path: &str,
        result: CiFetchResult,
        kind: CiFetchKind,
    ) {
        let abs = PathBuf::from(path);
        let (new_runs, github_total) = match result {
            CiFetchResult::Loaded { runs, github_total } => (runs, github_total),
            CiFetchResult::CacheOnly(runs) => (runs, 0),
        };

        let owner_paths = self
            .owner_repo_for_path_inner(&abs)
            .map(|repo| self.owner_paths_for_repo_inner(&repo))
            .filter(|paths| !paths.is_empty())
            .unwrap_or_else(|| vec![abs.clone()]);

        let prev_count = self
            .ci_state
            .get(&owner_paths[0])
            .map_or(0, |state| state.runs().len());

        let existing = self
            .ci_state
            .remove(&owner_paths[0])
            .map(|state| match state {
                CiState::Fetching { runs, .. } | CiState::Loaded { runs, .. } => runs,
            })
            .unwrap_or_default();

        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for run in new_runs {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        for run in existing {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        merged.sort_by(|left, right| right.run_id.cmp(&left.run_id));

        let found_new = merged.len() > prev_count;
        let exhausted = if found_new {
            if let Some(git) = self.git_info_for(&abs)
                && let Some(ref url) = git.url
                && let Some(owner_repo) = ci::parse_owner_repo(url)
            {
                scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
            }
            false
        } else {
            if let Some(git) = self.git_info_for(&abs)
                && let Some(ref url) = git.url
                && let Some(owner_repo) = ci::parse_owner_repo(url)
            {
                scan::mark_exhausted(owner_repo.owner(), owner_repo.repo());
            }
            if matches!(kind, CiFetchKind::Refresh) {
                self.status_flash =
                    Some(("no new runs found".to_string(), std::time::Instant::now()));
                self.show_timed_toast("CI", "No new runs found".to_string());
            }
            true
        };

        self.ci_pane.set_pos(merged.len());
        if let Some(repo) = self.owner_repo_for_path_inner(&abs) {
            let meta = crate::scan::load_cached_repo_data(&self.repo_fetch_cache, &repo)
                .and_then(|cached| cached.meta);
            crate::scan::store_cached_repo_data(
                &self.repo_fetch_cache,
                &repo,
                crate::scan::CachedRepoData {
                    runs: merged.clone(),
                    meta,
                    github_total,
                },
            );
        }
        for owner_path in owner_paths {
            self.ci_state.insert(
                owner_path,
                CiState::Loaded {
                    runs: merged.clone(),
                    exhausted,
                    github_total,
                },
            );
        }
        self.data_generation += 1;
    }

    pub(super) fn is_ci_owner_path(&self, path: &Path) -> bool {
        self.projects.iter().any(|item| {
            item.path() == path
                || match item {
                    crate::project::RootItem::Worktrees(
                        crate::project::WorktreeGroup::Workspaces { linked, .. },
                    ) => linked.iter().any(|l| l.path() == path),
                    crate::project::RootItem::Worktrees(
                        crate::project::WorktreeGroup::Packages { linked, .. },
                    ) => linked.iter().any(|l| l.path() == path),
                    _ => false,
                }
        })
    }

    pub(super) fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci_display_modes.get(path).copied().unwrap_or_default()
    }

    pub(super) fn ci_display_mode_label_for_inner(&self, path: &Path) -> &'static str {
        match self.ci_display_mode_for(path) {
            CiRunDisplayMode::BranchOnly => "branch",
            CiRunDisplayMode::All => "all",
        }
    }

    fn branch_only_ci_filter(&self, path: &Path) -> Option<&str> {
        let git = self.git_info_for(path)?;
        let branch = git.branch.as_deref()?;
        let default_branch = git.default_branch.as_deref()?;
        (branch != default_branch).then_some(branch)
    }

    pub(super) fn ci_toggle_available_for_inner(&self, path: &Path) -> bool {
        self.branch_only_ci_filter(path).is_some()
    }

    pub(super) fn toggle_ci_display_mode_for_inner(&mut self, path: &Path) {
        if !self.ci_toggle_available_for_inner(path) {
            self.ci_display_modes.remove(path);
            return;
        }
        let new_mode = match self.ci_display_mode_for(path) {
            CiRunDisplayMode::BranchOnly => CiRunDisplayMode::All,
            CiRunDisplayMode::All => CiRunDisplayMode::BranchOnly,
        };
        self.ci_display_modes.insert(path.to_path_buf(), new_mode);
        self.ci_pane.home();
        self.data_generation += 1;
        self.detail_generation += 1;
    }

    pub(super) fn ci_runs_for_display_inner(&self, path: &Path) -> Vec<CiRun> {
        let Some(state) = self.ci_state_for(path) else {
            return Vec::new();
        };
        let runs = state.runs();
        let Some(branch) = self.branch_only_ci_filter(path) else {
            return runs.to_vec();
        };
        if self.ci_display_mode_for(path) == CiRunDisplayMode::All {
            return runs.to_vec();
        }
        let filtered: Vec<CiRun> = runs
            .iter()
            .filter(|run| run.branch == branch)
            .cloned()
            .collect();
        if filtered.is_empty() {
            runs.to_vec()
        } else {
            filtered
        }
    }

    pub(super) fn latest_ci_run_for_path(&self, path: &Path) -> Option<&CiRun> {
        let state = self.ci_state_for(path)?;
        let runs = state.runs();
        let Some(branch) = self.branch_only_ci_filter(path) else {
            return runs.first();
        };
        if self.ci_display_mode_for(path) == CiRunDisplayMode::All {
            return runs.first();
        }
        runs.iter()
            .find(|run| run.branch == branch)
            .or_else(|| runs.first())
    }
}

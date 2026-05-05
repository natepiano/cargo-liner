use std::collections::HashSet;
use std::path::Path;

use crate::lint;
use crate::lint::RegisterProjectRequest;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::scan;
use crate::tui::app::App;
use crate::watcher;
use crate::watcher::WatcherMsg;

impl App {
    pub(super) fn respawn_watcher(&mut self) {
        let watch_roots = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let new_watcher = watcher::spawn_watcher(
            &watch_roots,
            self.background.bg_sender(),
            self.config().ci_run_count(),
            self.config().include_non_rust(),
            self.net.http_client(),
            self.lint.runtime_clone(),
            self.scan.metadata_store_handle(),
        );
        self.background.replace_watcher_sender(new_watcher);
    }
    pub fn register_existing_projects(&self) {
        self.projects().for_each_leaf(|item| {
            self.register_item_background_services(item);
        });
    }
    pub fn finish_watcher_registration_batch(&self) {
        let _ = self
            .background
            .send_watcher(WatcherMsg::InitialRegistrationComplete);
    }
    pub(super) fn respawn_watcher_and_register_existing_projects(&mut self) {
        self.respawn_watcher();
        self.register_existing_projects();
        self.finish_watcher_registration_batch();
    }
    pub fn refresh_lint_runs_from_disk(&mut self) {
        let mut paths = Vec::new();
        self.projects().for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            let runs = lint::read_history(path);
            if let Some(lr) = self.project_list.lint_at_path_mut(path) {
                lr.set_runs(runs, path);
            }
        }
        self.refresh_lint_cache_usage_from_disk();
    }
    pub(super) fn reload_lint_history(&mut self, project_path: &Path) {
        if !self.projects().is_rust_at_path(project_path) {
            return;
        }
        let runs = lint::read_history(project_path);
        if let Some(lr) = self.project_list.lint_at_path_mut(project_path) {
            lr.set_runs(runs, project_path);
        }
    }
    pub fn refresh_lint_cache_usage_from_disk(&mut self) {
        let cache_size_bytes = self
            .config
            .current()
            .lint
            .cache_size_bytes()
            .unwrap_or(None);
        self.lint
            .set_cache_usage(lint::retained_cache_usage(cache_size_bytes));
    }
    /// Collect root project paths and metadata for the lint runtime.
    pub(super) fn lint_runtime_root_entries(&self) -> Vec<(AbsolutePath, bool)> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();

        for entry in self.projects() {
            let items: Vec<(&AbsolutePath, bool)> = match &entry.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .map(|p| (p.path(), true))
                    .collect(),
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .map(|p| (p.path(), true))
                    .collect(),
                _ => vec![(entry.item.path(), entry.item.is_rust())],
            };
            for (path, is_rust) in items {
                let owned = path.clone();
                if seen.insert(owned.clone()) {
                    entries.push((owned, is_rust));
                }
            }
        }

        entries
    }
    pub fn lint_runtime_projects(&self) -> Vec<RegisterProjectRequest> {
        if !self.scan.is_complete() {
            return Vec::new();
        }
        self.lint_runtime_root_entries()
            .into_iter()
            .filter(|(path, _)| !self.projects().is_deleted(path))
            .map(|(abs_path, is_rust)| RegisterProjectRequest {
                project_label: project::home_relative_path(&abs_path),
                abs_path,
                is_rust,
            })
            .collect()
    }
    pub(super) fn sync_lint_runtime_projects(&self) {
        let Some(runtime) = self.lint.runtime() else {
            return;
        };
        runtime.sync_projects(self.lint_runtime_projects());
    }
    pub(super) fn register_lint_for_root_items(&self) -> usize {
        let Some(runtime) = self.lint.runtime() else {
            return 0;
        };
        let mut count = 0;
        for entry in self.projects() {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: ws.display_path().into_string(),
                        abs_path:      ws.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: pkg.display_path().into_string(),
                        abs_path:      pkg.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for ws in std::iter::once(primary).chain(linked.iter()) {
                        runtime.register_project(RegisterProjectRequest {
                            project_label: ws.display_path().into_string(),
                            abs_path:      ws.path().clone(),
                            is_rust:       true,
                        });
                        count += 1;
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    for pkg in std::iter::once(primary).chain(linked.iter()) {
                        runtime.register_project(RegisterProjectRequest {
                            project_label: pkg.display_path().into_string(),
                            abs_path:      pkg.path().clone(),
                            is_rust:       true,
                        });
                        count += 1;
                    }
                },
                RootItem::NonRust(_) => {},
            }
        }
        tracing::info!(count, "lint_register_root_items");
        count
    }
    pub(super) fn register_lint_project_if_eligible(&self, item: &RootItem) {
        if !item.is_rust() {
            tracing::info!(reason = "not_rust", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let path = item.path();
        // Skip workspace members — the workspace root's watcher covers them.
        let mut is_member = false;
        self.projects().for_each_leaf(|existing| {
            if matches!(
                &existing.item,
                RootItem::Rust(crate::project::RustProject::Workspace(_))
            ) && existing.item.path() != path
                && path.starts_with(existing.item.path())
            {
                is_member = true;
            }
        });
        if is_member {
            tracing::info!(reason = "workspace_member", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let Some(runtime) = self.lint.runtime() else {
            tracing::info!(reason = "no_runtime", path = %item.display_path(), "lint_register_skip");
            return;
        };
        tracing::info!(path = %item.display_path(), "lint_register");
        runtime.register_project(RegisterProjectRequest {
            project_label: item.display_path().into_string(),
            abs_path:      path.clone(),
            is_rust:       true,
        });
    }
    pub(super) fn register_lint_for_path(&self, path: &Path) {
        if let Some(item) = self.projects().iter().find(|i| i.path() == path) {
            self.register_lint_project_if_eligible(item);
        }
    }
}

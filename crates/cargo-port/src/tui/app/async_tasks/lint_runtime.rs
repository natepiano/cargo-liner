use std::path::Path;

use crate::lint;
use crate::lint::LintRun;
use crate::lint::RegisterProjectRequest;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::watcher;
use crate::watcher::WatcherMsg;

impl App {
    pub(super) fn respawn_watcher(&mut self) {
        let watch_roots = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let new_watcher = watcher::spawn_watcher(
            &watch_roots,
            self.background.background_sender(),
            self.config.ci_run_count(),
            self.config.include_non_rust(),
            self.net.http_client(),
            self.lint.runtime_clone(),
            self.scan.metadata_store_handle(),
        );
        self.background.replace_watcher_sender(new_watcher);
    }
    pub fn register_existing_projects(&self) {
        self.project_list.for_each_leaf(|item| {
            self.background.register_item_background_services(item);
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
    /// Load every Rust project's lint history off the main thread. Reading
    /// and JSON-parsing one history file per project synchronously freezes
    /// the first content paint for over a second on a large tree, so the
    /// reads run on the tokio blocking pool and land back as
    /// [`BackgroundMsg::LintHistoryLoaded`], applied by
    /// [`Self::apply_lint_history_loaded`].
    pub fn refresh_lint_runs_from_disk(&self) {
        let mut paths = Vec::new();
        self.project_list.for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(AbsolutePath::from(path.to_path_buf()));
            }
        });
        let tx = self.background.background_sender();
        let handle = self.net.http_client().handle;
        handle.spawn(async move {
            let entries = tokio::task::spawn_blocking(move || {
                paths
                    .into_iter()
                    .map(|path| {
                        let runs = lint::read_history(path.as_path());
                        (path, runs)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(BackgroundMsg::LintHistoryLoaded { entries });
        });
        self.refresh_lint_cache_usage_from_disk();
    }
    /// Apply lint history read off the main thread by
    /// [`Self::refresh_lint_runs_from_disk`], then invalidate the detail
    /// cache so the selected project's lint runs render.
    pub(super) fn apply_lint_history_loaded(&mut self, entries: Vec<(AbsolutePath, Vec<LintRun>)>) {
        for (path, runs) in entries {
            if let Some(lr) = self.project_list.lint_at_path_mut(path.as_path()) {
                lr.set_runs(runs);
            }
        }
        self.scan.bump_generation();
    }
    pub(super) fn reload_lint_history(&mut self, project_path: &Path) {
        if !self.project_list.is_rust_at_path(project_path) {
            return;
        }
        let runs = lint::read_history(project_path);
        if let Some(lr) = self.project_list.lint_at_path_mut(project_path) {
            lr.set_runs(runs);
        }
    }
    /// Spawn the lint-cache-size disk walk on the tokio blocking pool.
    /// The result lands on the main thread as
    /// [`BackgroundMsg::LintCacheUsage`] and is applied by
    /// [`crate::tui::state::Lint::set_cache_usage`]. Returns immediately
    /// so callers don't block the first paint (or any frame) on a walk
    /// of `~/Library/Caches/cargo-port/lint-runs`, which can hold
    /// thousands of archived run files.
    pub fn refresh_lint_cache_usage_from_disk(&self) {
        let cache_size_bytes = self
            .config
            .current()
            .lint
            .cache_size_bytes()
            .unwrap_or(None);
        let tx = self.background.background_sender();
        let handle = self.net.http_client().handle;
        handle.spawn(async move {
            let usage =
                tokio::task::spawn_blocking(move || lint::retained_cache_usage(cache_size_bytes))
                    .await
                    .unwrap_or_default();
            let _ = tx.send(BackgroundMsg::LintCacheUsage { usage });
        });
    }
    pub fn lint_runtime_projects(&self) -> Vec<RegisterProjectRequest> {
        if !self.scan.is_complete() {
            return Vec::new();
        }
        self.project_list
            .lint_runtime_root_entries()
            .into_iter()
            .filter(|(path, _)| !self.project_list.is_deleted(path))
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
    pub(super) fn register_lint_project_if_eligible(&self, item: &RootItem) {
        if !item.is_rust() {
            tracing::info!(reason = "not_rust", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let path = item.path();
        // Skip workspace members — the workspace root's watcher covers them.
        let mut is_member = false;
        self.project_list.for_each_leaf(|existing| {
            if matches!(&existing.item, RootItem::Rust(RustProject::Workspace(_)))
                && existing.item.path() != path
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
        if let Some(item) = self.project_list.iter().find(|i| i.path() == path) {
            self.register_lint_project_if_eligible(item);
        }
    }
}

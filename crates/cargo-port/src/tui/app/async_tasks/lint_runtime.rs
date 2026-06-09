use std::collections::HashSet;
use std::path::Path;

use crate::lint;
use crate::lint::CachedLintStatus;
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
    /// Every Rust leaf project's path — the set whose lint history is read
    /// from disk at startup. Drives both the off-thread load in
    /// [`Self::refresh_lint_runs_from_disk`] and the startup panel's "Lint
    /// history" row, so the row's denominator is identical to the set the
    /// load reports back and the row always completes.
    pub(super) fn lint_history_project_paths(&self) -> HashSet<AbsolutePath> {
        let mut paths = HashSet::new();
        self.project_list.for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.insert(AbsolutePath::from(path.to_path_buf()));
            }
        });
        paths
    }

    /// Load every Rust project's lint history off the main thread. Reading
    /// and JSON-parsing one history file per project synchronously freezes
    /// the first content paint for over a second on a large tree, so the
    /// reads run on the tokio blocking pool and land back as
    /// [`BackgroundMsg::LintHistoryLoaded`], applied by
    /// [`Self::apply_lint_history_loaded`].
    pub fn refresh_lint_runs_from_disk(&self) {
        let paths: Vec<AbsolutePath> = self.lint_history_project_paths().into_iter().collect();
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
    /// [`Self::refresh_lint_runs_from_disk`], mark the startup "Lint history"
    /// row's projects as loaded, then invalidate the detail cache so the
    /// selected project's lint runs render.
    pub(super) fn apply_lint_history_loaded(&mut self, entries: Vec<(AbsolutePath, Vec<LintRun>)>) {
        for (path, runs) in entries {
            if let Some(lr) = self.project_list.lint_at_path_mut(path.as_path()) {
                lr.set_hydrated_runs(runs);
            }
            self.startup.lint_phase.seen.insert(path);
        }
        self.scan.bump_generation();
        self.maybe_log_startup_phase_completions();
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

    /// As the startup phase closes, lint every eligible project whose source
    /// changed since its last run — or that was never linted, when discovery
    /// linting is enabled. Deferred to here rather than run during sync so
    /// these lints never contend with startup work. The newest source mtime
    /// per project comes from the disk walk (`Startup::source_mtimes`); the
    /// last run's start time comes from the loaded history. Surfaces the batch
    /// in a one-shot toast naming the projects.
    pub(super) fn kick_off_startup_lints(&mut self) {
        let Some(runtime) = self.lint.runtime().cloned() else {
            return;
        };
        let on_discovery = self.config.current().lint.on_discovery;

        let mut pending: Vec<AbsolutePath> = Vec::new();
        for request in self.lint_runtime_projects() {
            if !lint::project_is_eligible(
                &self.config.current().lint,
                &request.project_label,
                request.abs_path.as_path(),
                request.is_rust,
            ) {
                continue;
            }
            let Some(runs) = self.project_list.lint_at_path(request.abs_path.as_path()) else {
                continue;
            };
            // `from_lint_status` is `None` for a live `Running`/`Stale` status:
            // never re-trigger a project that is already linting.
            let Some(cached) = CachedLintStatus::from_lint_status(runs.status()) else {
                continue;
            };
            let last_started_at = runs.last_started_at();
            let max_source_mtime = self.startup.source_mtimes.get(&request.abs_path).copied();
            if cached.should_lint_on_startup(last_started_at, max_source_mtime, on_discovery) {
                pending.push(request.abs_path);
            }
        }

        self.startup.source_mtimes.clear();
        if pending.is_empty() {
            return;
        }

        // Title the single running-lint toast as a catch-up batch so it reads
        // distinctly from an edit-triggered run. That toast already names each
        // project and tracks elapsed/completion, so no separate one-shot toast
        // is needed.
        self.lint.queue_catch_up_lints();
        for path in &pending {
            runtime.request_startup_lint(path.clone());
        }
        tracing::info!(count = pending.len(), "startup_lints_kicked_off");
    }
}

use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use super::constants::PROJECT_STORAGE_REFRESH_SECS;
use crate::project::AbsolutePath;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Dismissed;
use crate::project::Visibility::Visible;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::DirSizes;
use crate::scan::ProjectStorage;
use crate::tui::app::App;

impl App {
    /// Refresh the mounted-volume capacity for the roots represented in the
    /// project-pane summary without doing filesystem work on the render path.
    pub(super) fn refresh_project_storage(&mut self) {
        self.storage_refresh_at = Instant::now();
        let paths = self.project_list.visible_project_storage_paths();
        let sender = self.background.background_sender();
        let handle = self.net.http_client().handle;
        handle.spawn(async move {
            let paths_for_probe = paths.clone();
            let storage =
                tokio::task::spawn_blocking(move || scan::project_storage(&paths_for_probe))
                    .await
                    .unwrap_or_default();
            let _ = sender.send(BackgroundMsg::ProjectStorage { paths, storage });
        });
    }

    /// Refresh free capacity periodically so non-project disk activity is
    /// reflected even when no watched path changes.
    pub(super) fn refresh_project_storage_if_due(&mut self, now: Instant) {
        if project_storage_refresh_due(self.storage_refresh_at, now) {
            self.refresh_project_storage();
        }
    }

    pub(super) fn set_project_storage(&mut self, paths: &[AbsolutePath], storage: ProjectStorage) {
        self.project_list.set_project_storage(paths, storage);
    }

    pub(in crate::tui::app) fn handle_disk_usage(&mut self, path: &Path, bytes: u64) {
        self.apply_disk_usage(path, bytes);
    }
    pub(super) fn handle_disk_usage_batch(&mut self, entries: Vec<(AbsolutePath, DirSizes)>) {
        for (path, sizes) in entries {
            self.apply_disk_usage_breakdown(path.as_path(), sizes);
        }
    }
    /// Apply a [`DirSizes`] breakdown to the matching project. Shares
    /// the post-set logic with `apply_disk_usage` (visibility /
    /// lint-runtime registration) by reusing that helper for the
    /// total — the breakdown fields are stored alongside.
    pub(super) fn apply_disk_usage_breakdown(&mut self, path: &Path, sizes: DirSizes) {
        if let Some(project) = self.project_list.at_path_mut(path) {
            project.in_project_target = Some(sizes.in_project_target);
            project.in_project_non_target = Some(sizes.in_project_non_target);
        }
        self.apply_disk_usage(path, sizes.total);
    }
    pub(super) fn apply_disk_usage(&mut self, path: &Path, bytes: u64) {
        // `project_list.at_path_mut` locates the `ProjectInfo` whose `disk_usage_bytes` and
        // `visibility` are updated to reflect `Deleted` or `Visible`; `lint_runtime_changed` tracks
        // required registration updates.
        let mut lint_runtime_changed = false;
        if let Some(project) = self.project_list.at_path_mut(path) {
            project.disk_usage_bytes = Some(bytes);
            // Periodic probes also include dismissed roots. Their result must
            // not undo the user's explicit dismissal of a deleted project.
            if project.visibility == Dismissed {
                return;
            }
            if bytes == 0 && !path.exists() && project.visibility != Deleted {
                project.visibility = Deleted;
                lint_runtime_changed = true;
            } else if bytes > 0 && project.visibility != Visible {
                project.visibility = Visible;
                lint_runtime_changed = true;
            }
        }
        if lint_runtime_changed {
            if let Some(runtime) = self.lint.runtime()
                && bytes == 0
            {
                runtime.unregister_project(AbsolutePath::from(path));
            }
            if bytes > 0 {
                self.register_lint_for_path(path);
            }
        }
    }
    pub(super) fn handle_disk_usage_msg(&mut self, path: &Path, bytes: u64) {
        self.startup.disk.seen.insert(AbsolutePath::from(path));
        self.handle_disk_usage(path, bytes);
        self.refresh_project_storage();
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn handle_disk_usage_batch_msg(
        &mut self,
        root_path: &AbsolutePath,
        entries: Vec<(AbsolutePath, DirSizes)>,
    ) {
        self.scan.bump_generation();
        // Insert root_path and each entries path into startup.disk.seen: group_disk_usage_trees
        // groups flat disk_entries under shallow ancestors while ProjectList::iter follows
        // build_tree workspaces/worktree groups, so root-only marking could leave expected paths
        // permanently pending; untracked keys are harmless.
        self.startup.disk.seen.insert(root_path.clone());
        for (path, sizes) in &entries {
            if path != root_path {
                self.startup.disk.seen.insert(path.clone());
            }
            if let Some(mtime) = sizes.max_source_mtime {
                self.startup.source_mtimes.insert(path.clone(), mtime);
            }
        }
        self.handle_disk_usage_batch(entries);
        self.refresh_project_storage();
        self.maybe_log_startup_phase_completions();
    }
}

fn project_storage_refresh_due(last_refresh: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_refresh) >= Duration::from_secs(PROJECT_STORAGE_REFRESH_SECS)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::PROJECT_STORAGE_REFRESH_SECS;
    use super::project_storage_refresh_due;
    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::Visibility;
    use crate::tui::test_support;

    #[test]
    #[allow(clippy::expect_used, reason = "fixture paths must be present")]
    fn disk_poll_preserves_deleted_rows_and_explicit_dismissal() {
        let scratch = tempfile::tempdir().expect("scratch project");
        let path = AbsolutePath::from(scratch.path().join("removed-project"));
        let item = RootItem::Rust(RustProject::Package(Package {
            path: path.clone(),
            ..Package::default()
        }));
        let mut app = test_support::make_app(&[item]);
        app.apply_disk_usage(&path, 0);
        assert_eq!(
            app.project_list
                .at_path(&path)
                .expect("deleted row remains")
                .visibility,
            Visibility::Deleted
        );
        app.project_list
            .at_path_mut(&path)
            .expect("row to dismiss")
            .visibility = Visibility::Dismissed;
        app.apply_disk_usage(&path, 0);
        assert_eq!(
            app.project_list
                .at_path(&path)
                .expect("dismissed row remains stored")
                .visibility,
            Visibility::Dismissed
        );
        app.apply_disk_usage(&path, 42);
        assert_eq!(
            app.project_list
                .at_path(&path)
                .expect("late result retains dismissal")
                .visibility,
            Visibility::Dismissed
        );
    }

    #[test]
    fn project_storage_refresh_is_due_after_two_seconds() {
        let start = Instant::now();
        let interval = Duration::from_secs(PROJECT_STORAGE_REFRESH_SECS);

        assert!(!project_storage_refresh_due(
            start,
            start + interval.saturating_sub(Duration::from_millis(1))
        ));
        assert!(project_storage_refresh_due(start, start + interval));
    }
}

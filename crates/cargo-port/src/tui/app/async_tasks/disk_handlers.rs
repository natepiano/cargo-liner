use std::path::Path;

use crate::project::AbsolutePath;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Visible;
use crate::scan::DirSizes;
use crate::tui::app::App;
use crate::tui::integration;

impl App {
    pub fn handle_disk_usage(&mut self, path: &Path, bytes: u64) {
        if self.inflight.clean_mut().remove(path).is_some() {
            self.sync_running_clean_toast();
        }
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
    /// total — the new breakdown fields just ride alongside.
    pub(super) fn apply_disk_usage_breakdown(&mut self, path: &Path, sizes: DirSizes) {
        if let Some(project) = self.project_list.at_path_mut(path) {
            project.in_project_target = Some(sizes.in_project_target);
            project.in_project_non_target = Some(sizes.in_project_non_target);
        }
        self.apply_disk_usage(path, sizes.total);
    }
    pub(super) fn apply_disk_usage(&mut self, path: &Path, bytes: u64) {
        // Set disk usage on the matching project item and update visibility.
        let mut lint_runtime_changed = false;
        if let Some(project) = self.project_list.at_path_mut(path) {
            project.disk_usage_bytes = Some(bytes);
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
        let abs = AbsolutePath::from(path);
        self.startup.disk.seen.insert(abs.clone());
        if let Some(disk_toast) = self.startup.disk.toast {
            let key = integration::path_key(&abs);
            self.framework.toasts.mark_item_completed(disk_toast, &key);
        }
        self.handle_disk_usage(path, bytes);
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn handle_disk_usage_batch_msg(
        &mut self,
        root_path: &AbsolutePath,
        entries: Vec<(AbsolutePath, DirSizes)>,
    ) {
        self.scan.bump_generation();
        self.startup.disk.seen.insert(root_path.clone());
        if let Some(disk_toast) = self.startup.disk.toast {
            let key = integration::path_key(root_path);
            self.framework.toasts.mark_item_completed(disk_toast, &key);
        }
        self.handle_disk_usage_batch(entries);
        self.maybe_log_startup_phase_completions();
    }
}

use std::time::Duration;
use std::time::Instant;

use crate::project;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::panes::PaneId;
use crate::tui::toasts::ToastStyle::Warning;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::ToastView;
use crate::tui::toasts::TrackedItem;

impl App {
    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.config.current().tui.status_flash_secs)
    }

    pub fn focused_toast_id(&self) -> Option<u64> {
        let active = self.toasts.active_now();
        active
            .get(self.panes().toasts().viewport().pos())
            .map(ToastView::id)
    }

    pub fn prune_toasts(&mut self) {
        let now = Instant::now();
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.prune_tracked_items(now, linger);
        self.toasts.prune(now);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
        if self.base_focus() == PaneId::Toasts && self.toasts.active_now().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout(), 1);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn show_timed_warning_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts
            .push_timed_styled(title, body, self.toast_timeout(), 1, Warning);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body, 1);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
        task_id
    }

    pub fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        let linger = if self.toasts.tracked_item_count(task_id) > 0 {
            Duration::from_secs_f64(self.config.current().tui.task_linger_secs)
        } else {
            Duration::ZERO
        };
        self.toasts.finish_task(task_id, linger);
        self.prune_toasts();
    }

    pub fn set_task_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) {
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.set_tracked_items(task_id, items, linger);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) {
        self.toasts.mark_item_completed(task_id, key);
        let toast_len = self.toasts.active_now().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    /// Begin a clean for `project_path`. Returns `true` if a cargo clean
    /// should be spawned; `false` when the project is already clean,
    /// in which case a timed "Already clean" toast is shown and no
    /// spinner is started.
    pub fn start_clean(&mut self, project_path: &AbsolutePath) -> bool {
        let target_dir = self
            .scan
            .resolve_target_dir(project_path)
            .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join("target")));
        if !target_dir.as_path().exists() {
            let name = project::home_relative_path(project_path.as_path());
            self.show_timed_toast("Already clean", name);
            return false;
        }
        self.inflight
            .clean_mut()
            .insert(project_path.clone(), Instant::now());
        self.sync_running_clean_toast();
        true
    }

    pub fn clean_spawn_failed(&mut self, project_path: &AbsolutePath) {
        self.inflight.clean_mut().remove(project_path.as_path());
        self.sync_running_clean_toast();
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }
}

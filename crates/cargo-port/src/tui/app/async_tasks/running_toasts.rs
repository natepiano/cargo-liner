use std::collections::HashSet;
use std::time::Duration;

use crate::project;
use crate::tui::app::App;
use crate::tui::toasts;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::TrackedItem;

impl App {
    pub fn sync_running_clean_toast(&mut self) {
        let (toast_slot, items) = self
            .inflight
            .clean()
            .items_for_toast(|p| project::home_relative_path(p.as_path()));
        let next = self.sync_running_toast(toast_slot, "cargo clean", &items[..]);
        self.inflight.clean_mut().toast = next;
    }
    pub(super) fn sync_running_lint_toast(&mut self) {
        let (toast_slot, items) = self
            .lint
            .running()
            .items_for_toast(|p| project::home_relative_path(p.as_path()));
        let next = self.sync_running_toast(toast_slot, "Lints", &items);
        self.lint.running_mut().toast = next;
    }
    /// Keep a single "Retrieving GitHub repo details" toast in sync
    /// with the live in-flight repo fetches.
    pub(super) fn sync_running_repo_fetch_toast(&mut self) {
        let (toast_slot, items) = self
            .net
            .github
            .running()
            .items_for_toast(ToString::to_string);
        let next = self.sync_running_toast(toast_slot, "Retrieving GitHub repo details", &items);
        self.net.github.running_mut().toast = next;
    }
    /// Shared tracked-task toast sync. Grows as new items appear,
    /// marks items completed (freezing elapsed + starting strikethrough)
    /// as items disappear, and begins the toast-level linger
    /// countdown once the tracker drains. Used by lint, clean, and
    /// GitHub repo-fetch flows via [`RunningTracker::items_for_toast`].
    pub(super) fn sync_running_toast(
        &mut self,
        toast_slot: Option<ToastTaskId>,
        title: &str,
        running_items: &[TrackedItem],
    ) -> Option<ToastTaskId> {
        if running_items.is_empty() {
            if let Some(task_id) = toast_slot {
                let empty: HashSet<String> = HashSet::new();
                self.toasts.complete_missing_items(task_id, &empty);
                if !self.toasts.is_task_finished(task_id) {
                    let linger =
                        Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
                    self.toasts.finish_task(task_id, linger);
                }
            }
            return toast_slot;
        }

        let running_keys: HashSet<String> = running_items
            .iter()
            .map(|item| item.key.to_string())
            .collect();

        if let Some(task_id) = toast_slot
            && self.toasts.reactivate_task(task_id)
        {
            self.toasts.complete_missing_items(task_id, &running_keys);
            let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
            self.toasts
                .add_new_tracked_items(task_id, running_items, linger);
            for item in running_items {
                if let Some(started) = item.started_at {
                    self.toasts
                        .restart_tracked_item(task_id, &item.key, started);
                }
            }
            Some(task_id)
        } else {
            let labels: Vec<&str> = running_items.iter().map(|i| i.label.as_str()).collect();
            let body = toasts::format_toast_items(&labels, toasts::toast_body_width());
            let task_id = self.toasts.start_task(title, body);
            self.set_task_tracked_items(task_id, running_items);
            Some(task_id)
        }
    }
}

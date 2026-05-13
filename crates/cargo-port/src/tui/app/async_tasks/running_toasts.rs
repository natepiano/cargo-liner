use std::collections::HashSet;

use tui_pane::ToastTaskId;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemKey;
use tui_pane::format_toast_items;
use tui_pane::toast_body_width;

use crate::project;
use crate::tui::app::App;
use crate::tui::integration::toast_adapters;

impl App {
    pub fn sync_running_clean_toast(&mut self) {
        let (toast_slot, items) = self.inflight.clean().items_for_toast(
            |p| project::home_relative_path(p.as_path()),
            toast_adapters::path_key,
        );
        let next = self.sync_running_toast(toast_slot, "cargo clean", &items[..]);
        self.inflight.clean_mut().toast = next;
    }
    pub(super) fn sync_running_lint_toast(&mut self) {
        let (toast_slot, items) = self.lint.running().items_for_toast(
            |p| project::home_relative_path(p.as_path()),
            toast_adapters::path_key,
        );
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
            .items_for_toast(ToString::to_string, toast_adapters::owner_repo_key);
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
                let empty: HashSet<TrackedItemKey> = HashSet::new();
                self.framework
                    .toasts
                    .complete_missing_items(task_id, &empty);
                if !self.framework.toasts.is_task_finished(task_id) {
                    let linger = self.framework.toast_settings().task_linger.get();
                    self.framework.toasts.finish_task(task_id, linger);
                }
            }
            return toast_slot;
        }

        let running_keys: HashSet<TrackedItemKey> =
            running_items.iter().map(|item| item.key.clone()).collect();

        if let Some(task_id) = toast_slot
            && self.framework.toasts.reactivate_task(task_id)
        {
            self.framework
                .toasts
                .complete_missing_items(task_id, &running_keys);
            let linger = self.framework.toast_settings().task_linger.get();
            self.framework
                .toasts
                .add_new_tracked_items(task_id, running_items, linger);
            for item in running_items {
                if let Some(started) = item.started_at {
                    self.framework
                        .toasts
                        .restart_tracked_item(task_id, &item.key, started);
                }
            }
            Some(task_id)
        } else {
            let labels: Vec<&str> = running_items.iter().map(|i| i.label.as_str()).collect();
            let width = toast_body_width(self.framework.toast_settings());
            let body = format_toast_items(&labels, width);
            let task_id = self.framework.toasts.start_task(title, body);
            self.set_task_tracked_items(task_id, running_items);
            Some(task_id)
        }
    }
}

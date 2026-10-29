use std::collections::HashSet;

use tui_pane::RunningToastSlot;
use tui_pane::ToastTaskId;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemKey;

use crate::lint::LintRunOrigin;
use crate::project;
use crate::tui::app::App;
use crate::tui::integration;

impl App {
    pub(in crate::tui::app) fn sync_running_clean_toast(&mut self) {
        let (toast_slot, items) = self.inflight.clean().items_for_toast(
            |p| project::home_relative_path(p.as_path()),
            integration::path_key,
        );
        let next = self.sync_running_toast(toast_slot, "cargo clean", &items[..]);
        self.inflight.clean_mut().toast = next;
    }
    pub(super) fn sync_running_lint_toast(&mut self) {
        self.sync_running_lint_toast_for_origin(LintRunOrigin::CatchUp);
        self.sync_running_lint_toast_for_origin(LintRunOrigin::Normal);
    }
    fn sync_running_lint_toast_for_origin(&mut self, origin: LintRunOrigin) {
        let (toast_slot, items) = self.lint.toast_items_for_origin(origin);
        let title = crate::tui::state::Lint::running_toast_title(origin);
        let next = self.sync_running_toast(toast_slot, title, &items);
        self.lint.set_running_toast_for_origin(origin, next);
    }
    /// Keep a single "Retrieving GitHub repo details" toast in sync with the
    /// live in-flight repo fetches. The toast slot lives only in the
    /// steady-state network-toast stage: while the startup panel owns the
    /// GitHub row, `Net::network_toasts` returns `None`, so there is nowhere
    /// to store a toast id and this no-ops.
    pub(super) fn sync_running_repo_fetch_toast(&mut self) {
        let Some(toast_slot) = self.net.network_toasts().map(|toasts| toasts.github) else {
            return;
        };
        let (_, items) = self
            .net
            .github_running()
            .items_for_toast(ToString::to_string, integration::owner_repo_key);
        let next = self.sync_running_toast(toast_slot, "Retrieving GitHub repo details", &items);
        if let Some(toasts) = self.net.network_toasts_mut() {
            toasts.github = next;
        }
    }
    /// Keep a single "Fetching crates.io info" toast in sync with the live
    /// in-flight crates.io fetches. Mirrors the GitHub repo-fetch toast: the
    /// slot exists only in steady state, so while the startup panel owns the
    /// crates.io row this no-ops and no standalone toast can be created.
    pub(super) fn sync_running_crates_io_toast(&mut self) {
        let Some(toast_slot) = self.net.network_toasts().map(|toasts| toasts.crates_io) else {
            return;
        };
        let (_, items) = self
            .net
            .crates_io_running()
            .items_for_toast(String::clone, |name| TrackedItemKey::from(name.as_str()));
        let next = self.sync_running_toast(toast_slot, "Fetching crates.io info", &items);
        if let Some(toasts) = self.net.network_toasts_mut() {
            toasts.crates_io = next;
        }
    }
    /// Return the network-toast stage to `StartupOwned`: finish any live
    /// standalone GitHub / crates.io toasts (their slots are about to be
    /// discarded, so their ids would otherwise strand), then drop the slots.
    /// Called when a rescan re-opens the consolidated panel, which takes back
    /// ownership of those rows.
    pub(super) fn enter_startup_owned_network_stage(&mut self) {
        if let Some(slots) = self.net.network_toasts().map(|t| [t.crates_io, t.github]) {
            for task_id in slots.into_iter().flatten() {
                self.framework
                    .toasts
                    .complete_missing_items(task_id, &HashSet::new());
            }
        }
        self.net.set_network_toasts_startup_owned();
    }
    /// Reconcile a task toast using the framework policy.
    pub(super) fn sync_running_toast(
        &mut self,
        toast_slot: Option<ToastTaskId>,
        title: &str,
        running_items: &[TrackedItem],
    ) -> Option<ToastTaskId> {
        let width = tui_pane::toast_body_width(self.framework.toast_settings());
        let slot = toast_slot.map_or(RunningToastSlot::Vacant, RunningToastSlot::Assigned);
        match tui_pane::sync_running_task(
            &mut self.framework.toasts,
            slot,
            title,
            running_items,
            width,
        ) {
            RunningToastSlot::Vacant => None,
            RunningToastSlot::Assigned(task_id) => Some(task_id),
        }
    }
}

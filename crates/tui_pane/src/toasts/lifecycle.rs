use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use super::ReactivateOutcome;
use super::Toast;
use super::ToastBody;
use super::ToastTaskId;
use super::ToastView;
use super::Toasts;
use super::TrackedItem;
use super::TrackedItemKey;
use super::toast::ToastDismissal;
use super::toast::ToastLifetime;
use super::toast::ToastPhase;
use super::toast::ToastTaskStatus;
use crate::AppContext;

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Return whether any live, non-exiting toast is active.
    #[must_use]
    pub fn has_active(&self) -> bool {
        let now = Instant::now();
        self.entries.iter().any(|toast| toast.is_live(now))
    }

    /// Return all stored toast entries.
    #[must_use]
    pub fn active(&self) -> &[Toast<Ctx>] { &self.entries }

    /// Return renderable toast views at the current instant.
    #[must_use]
    pub fn active_now(&self) -> Vec<ToastView> { self.active_views(Instant::now()) }

    /// Return renderable toast views at `now`.
    #[must_use]
    pub fn active_views(&self, now: Instant) -> Vec<ToastView> {
        self.entries
            .iter()
            .filter(|toast| toast.is_renderable(now, self.settings()))
            .map(|toast| toast.view(now, self.settings()))
            .collect()
    }

    /// Mark a task toast as finished.
    ///
    /// For a toast with tracked items, this marks any still-incomplete
    /// item as completed at `now` and then runs [`Self::recompute_task_status`],
    /// which derives `finished_at = max(item.completed_at)`. That
    /// anchoring makes the "Closing in N" countdown coincide exactly
    /// with the last item's individual `item_linger`.
    ///
    /// For a toast with no items, transitions directly to
    /// [`ToastTaskStatus::Finished`] with `Duration::ZERO` linger —
    /// the toast closes on the next prune pass.
    pub fn finish_task(&mut self, task_id: ToastTaskId) -> bool {
        let now = Instant::now();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        if !matches!(toast.lifetime, ToastLifetime::Task { .. }) {
            return false;
        }
        if toast.tracked_items.is_empty() {
            toast.lifetime = ToastLifetime::Task {
                task_id,
                status: ToastTaskStatus::Finished {
                    finished_at: now,
                    linger:      Duration::ZERO,
                },
            };
            return true;
        }
        for item in &mut toast.tracked_items {
            if item.completed_at.is_none() {
                item.completed_at = Some(now);
            }
        }
        self.recompute_task_status(task_id);
        true
    }

    /// Mark a finished task toast as running again — unless the
    /// user explicitly dismissed it during this tracker session.
    ///
    /// Returns a [`ReactivateOutcome`] so the caller can
    /// distinguish "no toast for this task — create one" from
    /// "user dismissed this toast — leave it alone." The third
    /// outcome, [`ReactivateOutcome::Revived`], is the original
    /// reactivation path.
    pub fn reactivate_task(&mut self, task_id: ToastTaskId) -> ReactivateOutcome {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return ReactivateOutcome::NotFound;
        };
        if matches!(toast.dismissal, ToastDismissal::ClosedByUser) {
            return ReactivateOutcome::DismissedByUser;
        }
        toast.lifetime = ToastLifetime::Task {
            task_id,
            status: ToastTaskStatus::Running,
        };
        toast.phase = ToastPhase::Visible;
        ReactivateOutcome::Revived
    }

    /// Replace the body text for a task toast.
    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.body = ToastBody::from(body.into());
        true
    }

    /// Replace the tracked-item list for a task toast. Item linger
    /// is read from
    /// [`ToastSettings::finished_task_visible`](crate::ToastSettings::finished_task_visible).
    /// After replacement, the toast's lifetime status is recomputed
    /// — replacing the list with incomplete items reverts a
    /// previously-finished toast back to running.
    pub fn set_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) -> bool {
        let linger = self.settings().finished_task_visible.get();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.tracked_items = items.to_vec();
        toast.item_linger = linger;
        self.recompute_task_status(task_id);
        true
    }

    /// Return whether the toast is currently live.
    #[must_use]
    pub fn is_alive(&self, id: super::ToastId) -> bool {
        let now = Instant::now();
        self.entries
            .iter()
            .find(|toast| toast.id == id)
            .is_some_and(|toast| toast.is_live(now))
    }

    /// Return whether the task toast is in the finished state.
    #[must_use]
    pub fn is_task_finished(&self, task_id: ToastTaskId) -> bool {
        self.toast_for_task(task_id).is_some_and(|toast| {
            matches!(
                toast.lifetime,
                ToastLifetime::Task {
                    status: ToastTaskStatus::Finished { .. },
                    ..
                }
            )
        })
    }

    /// Return the number of tracked items on a task toast.
    #[must_use]
    pub fn tracked_item_count(&self, task_id: ToastTaskId) -> usize {
        self.toast_for_task(task_id)
            .map(|toast| toast.tracked_items.len())
            .unwrap_or_default()
    }

    /// Mark tracked items missing from `active_keys` as completed.
    pub fn complete_missing_items(
        &mut self,
        task_id: ToastTaskId,
        active_keys: &HashSet<TrackedItemKey>,
    ) -> bool {
        let now = Instant::now();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let mut changed = false;
        for item in &mut toast.tracked_items {
            if item.completed_at.is_none() && !active_keys.contains(&item.key) {
                item.completed_at = Some(now);
                changed = true;
            }
        }
        if changed {
            self.recompute_task_status(task_id);
        }
        changed
    }

    /// Add tracked items whose keys are not already present. Item
    /// linger is read from
    /// [`ToastSettings::finished_task_visible`](crate::ToastSettings::finished_task_visible).
    /// After insertion, the toast's lifetime status is recomputed —
    /// a previously-finished toast reverts to running so the
    /// countdown re-anchors when the new items complete.
    pub fn add_new_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) -> bool {
        let item_linger = self.settings().finished_task_visible.get();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let existing_keys: HashSet<TrackedItemKey> = toast
            .tracked_items
            .iter()
            .map(|item| item.key.clone())
            .collect();
        let mut changed = false;
        for item in items {
            if !existing_keys.contains(&item.key) {
                toast.tracked_items.push(item.clone());
                changed = true;
            }
        }
        toast.item_linger = item_linger;
        if changed {
            self.recompute_task_status(task_id);
        }
        changed
    }

    /// Restart one tracked item by key. After clearing the item's
    /// `completed_at`, the toast's lifetime status is recomputed —
    /// a previously-finished toast reverts to running.
    pub fn restart_tracked_item(
        &mut self,
        task_id: ToastTaskId,
        key: &TrackedItemKey,
        now: Instant,
    ) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let Some(item) = toast.tracked_items.iter_mut().find(|item| item.key == *key) else {
            return false;
        };
        item.started_at = Some(now);
        item.completed_at = None;
        self.recompute_task_status(task_id);
        true
    }

    /// Mark one tracked item completed by key.
    pub fn mark_item_completed(&mut self, task_id: ToastTaskId, key: &TrackedItemKey) -> bool {
        let now = Instant::now();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let Some(item) = toast.tracked_items.iter_mut().find(|item| item.key == *key) else {
            return false;
        };
        item.completed_at = Some(now);
        self.recompute_task_status(task_id);
        true
    }

    /// Mark one tracked item completed by string key.
    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) -> bool {
        self.mark_item_completed(task_id, &TrackedItemKey::new(key))
    }

    /// Drop completed tracked items whose linger duration has elapsed.
    /// Linger is read from
    /// [`ToastSettings::finished_task_visible`](crate::ToastSettings::finished_task_visible).
    pub fn prune_tracked_items(&mut self, now: Instant) {
        let linger = self.settings().finished_task_visible.get();
        for toast in &mut self.entries {
            if !matches!(toast.lifetime, ToastLifetime::Task { .. }) {
                continue;
            }
            toast.tracked_items.retain(|item| {
                item.completed_at
                    .is_none_or(|completed_at| now.saturating_duration_since(completed_at) < linger)
            });
        }
    }

    /// Advance toast lifetimes and remove entries whose exit animation is done.
    pub fn prune(&mut self, now: Instant) {
        for toast in &mut self.entries {
            if matches!(toast.phase, ToastPhase::Visible) && toast.should_exit(now) {
                toast.phase = ToastPhase::Exiting { started_at: now };
            }
        }
        let settings = &self.settings;
        self.entries
            .retain(|toast| toast.is_renderable(now, settings));
        self.sync_viewport_len();
    }

    /// Recompute a task toast's lifetime status from its tracked-item
    /// state, then call this after every item mutation. Invariant:
    /// the toast is `Finished` if and only if every tracked item
    /// has `completed_at = Some(_)`, with `finished_at = max(item.completed_at)`.
    ///
    /// * All items have `completed_at` → transition to `Finished` with `finished_at` anchored to
    ///   the latest completion. This makes the countdown's expiry coincide with the last item's
    ///   individual `item_linger` end.
    /// * Any item is still incomplete → transition (or stay) in `Running`. Reverts a
    ///   previously-finished toast back to running when new incomplete items arrive, so the
    ///   countdown re-anchors when the new work completes.
    /// * Zero items → no transition. A brand-new task toast with no items added yet stays in its
    ///   initial `Running` state until the embedding either adds items or calls
    ///   [`Self::finish_task`] explicitly.
    ///
    /// `phase` is also snapped back to `Visible` whenever the lifetime
    /// status reverts to `Running`, so an already-Exiting toast (only
    /// reachable for `Timed`/`Persistent` lifetimes — task toasts
    /// skip the exit animation) cannot get stuck mid-animation when
    /// its lifetime changes.
    pub(super) fn recompute_task_status(&mut self, task_id: ToastTaskId) {
        let Some(toast) = self.toast_for_task(task_id) else {
            return;
        };
        if !matches!(toast.lifetime, ToastLifetime::Task { .. }) {
            return;
        }
        if toast.tracked_items.is_empty() {
            return;
        }
        let all_completed = toast
            .tracked_items
            .iter()
            .all(|item| item.completed_at.is_some());
        let latest_completion = toast
            .tracked_items
            .iter()
            .filter_map(|item| item.completed_at)
            .max();
        let linger = self.settings().finished_task_visible.get();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return;
        };
        if all_completed {
            let finished_at = latest_completion.unwrap_or_else(Instant::now);
            toast.lifetime = ToastLifetime::Task {
                task_id,
                status: ToastTaskStatus::Finished {
                    finished_at,
                    linger,
                },
            };
        } else {
            toast.lifetime = ToastLifetime::Task {
                task_id,
                status: ToastTaskStatus::Running,
            };
            toast.phase = ToastPhase::Visible;
        }
    }

    pub(super) fn toast_for_task(&self, task_id: ToastTaskId) -> Option<&Toast<Ctx>> {
        self.entries
            .iter()
            .find(|toast| toast.task_id() == Some(task_id))
    }

    pub(super) fn toast_for_task_mut(&mut self, task_id: ToastTaskId) -> Option<&mut Toast<Ctx>> {
        self.entries
            .iter_mut()
            .find(|toast| toast.task_id() == Some(task_id))
    }
}

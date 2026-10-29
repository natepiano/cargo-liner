//! Reconcile tracked task toasts without application policy.

use std::collections::HashSet;

use super::ReactivateOutcome;
use super::ToastTaskId;
use super::Toasts;
use super::TrackedItem;
use super::TrackedItemKey;
use super::TrackedItemTiming;
use super::format_toast_items;
use crate::AppContext;

/// The toast assigned to one tracked task session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningToastSlot {
    /// No toast has been created for this session.
    Vacant,
    /// Keep this task id even when the user dismisses its toast.
    Assigned(ToastTaskId),
}

/// Reconcile a tracked task's toast with its current items.
///
/// Grows as new items appear,
/// marks items completed (freezing elapsed + starting strikethrough)
/// as items disappear, and begins the toast-level linger
/// countdown once the tracker drains. Used by lint, clean, and
/// GitHub repo-fetch flows via
/// [`RunningTracker::items_for_toast`](crate::RunningTracker::items_for_toast).
pub fn sync_running_task<Ctx: AppContext>(
    toasts: &mut Toasts<Ctx>,
    toast_slot: RunningToastSlot,
    title: &str,
    running_items: &[TrackedItem],
    body_width: usize,
) -> RunningToastSlot {
    if running_items.is_empty() {
        if let RunningToastSlot::Assigned(task_id) = toast_slot {
            let empty: HashSet<TrackedItemKey> = HashSet::new();
            // Marking every item completed via `complete_missing_items`
            // triggers the framework's auto-finish path — no explicit
            // `finish_task` needed.
            toasts.complete_missing_items(task_id, &empty);
        }
        return toast_slot;
    }

    let running_keys: HashSet<TrackedItemKey> =
        running_items.iter().map(|item| item.key.clone()).collect();

    match toast_slot {
        RunningToastSlot::Assigned(task_id) => match toasts.reactivate_task(task_id) {
            ReactivateOutcome::Revived => {
                toasts.complete_missing_items(task_id, &running_keys);
                toasts.add_new_tracked_items(task_id, running_items);
                // `add_new_tracked_items` skips keys the toast already
                // holds, so an item that stalls (or resumes) after it was
                // added needs this to reach the rendered spinner.
                toasts.refresh_tracked_item_activity(task_id, running_items);
                for item in running_items {
                    if let TrackedItemTiming::Running {
                        started_at: started,
                    }
                    | TrackedItemTiming::Completed {
                        started_at: started,
                        ..
                    } = item.timing
                    {
                        toasts.restart_tracked_item(task_id, &item.key, started);
                    }
                }
                RunningToastSlot::Assigned(task_id)
            },
            ReactivateOutcome::DismissedByUser => {
                // `ReactivateOutcome::DismissedByUser` retains `toast_slot`'s `task_id`, so
                // `sync_running_task` cannot create a duplicate next frame. Leave
                // `toasts` untouched while the tracker continues running and its UI
                // remains dismissed.
                RunningToastSlot::Assigned(task_id)
            },
            ReactivateOutcome::NotFound => RunningToastSlot::Assigned(start_running_toast(
                toasts,
                title,
                running_items,
                body_width,
            )),
        },
        RunningToastSlot::Vacant => RunningToastSlot::Assigned(start_running_toast(
            toasts,
            title,
            running_items,
            body_width,
        )),
    }
}

fn start_running_toast<Ctx: AppContext>(
    toasts: &mut Toasts<Ctx>,
    title: &str,
    running_items: &[TrackedItem],
    body_width: usize,
) -> ToastTaskId {
    let labels: Vec<&str> = running_items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    let body = format_toast_items(&labels, body_width);
    let task_id = toasts.start_task(title, body);
    toasts.set_tracked_items(task_id, running_items);
    task_id
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    struct TestApp {
        framework: crate::Framework<Self>,
    }

    impl AppContext for TestApp {
        type AppPaneId = ();
        type ToastAction = crate::NoToastAction;

        fn framework(&self) -> &crate::Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut crate::Framework<Self> { &mut self.framework }
    }

    use crate::ToastId;
    use crate::TrackedItemActivity;

    const DISMISSED_ITEM: &str = "dismissed";
    const FIRST_ITEM: &str = "first";
    const RUNNING_TOAST_TITLE: &str = "Running task";
    const SECOND_ITEM: &str = "second";
    const STALE_ITEM: &str = "stale";
    const STALE_TASK_ID: ToastTaskId = ToastTaskId(u64::MAX);

    #[test]
    fn sync_running_toast_preserves_reactivate_outcomes() {
        let mut toasts = Toasts::<TestApp>::new();
        let first_items = [tracked_item(FIRST_ITEM)];

        let first_task_id =
            sync_running_toast(&mut toasts, None, RUNNING_TOAST_TITLE, &first_items)
                .expect("running items should create the first toast");
        assert_eq!(toasts.active().len(), first_items.len());
        assert_eq!(toasts.tracked_item_count(first_task_id), first_items.len());

        let revived_items = [tracked_item(FIRST_ITEM), tracked_item(SECOND_ITEM)];
        let revived_task_id = sync_running_toast(
            &mut toasts,
            Some(first_task_id),
            RUNNING_TOAST_TITLE,
            &revived_items,
        )
        .expect("existing toast should reactivate");
        assert_eq!(revived_task_id, first_task_id);
        assert_eq!(toasts.active().len(), first_items.len());
        assert_eq!(
            toasts.tracked_item_count(first_task_id),
            revived_items.len()
        );

        assert!(toasts.dismiss(ToastId(first_task_id.get())));
        let stored_toasts_after_dismiss = toasts.active().len();
        let dismissed_items = [tracked_item(DISMISSED_ITEM)];
        let dismissed_task_id = sync_running_toast(
            &mut toasts,
            Some(first_task_id),
            RUNNING_TOAST_TITLE,
            &dismissed_items,
        )
        .expect("dismissed task slot should stay wired");
        assert_eq!(dismissed_task_id, first_task_id);
        assert_eq!(toasts.active().len(), stored_toasts_after_dismiss);
        assert_eq!(toasts.active_now().len(), 0);
        assert_eq!(
            toasts.tracked_item_count(first_task_id),
            revived_items.len()
        );

        let stale_items = [tracked_item(STALE_ITEM)];
        let fresh_task_id = sync_running_toast(
            &mut toasts,
            Some(STALE_TASK_ID),
            RUNNING_TOAST_TITLE,
            &stale_items,
        )
        .expect("stale task slot should create a replacement toast");
        assert_ne!(fresh_task_id, STALE_TASK_ID);
        assert_ne!(fresh_task_id, first_task_id);
        assert_eq!(toasts.active().len(), stored_toasts_after_dismiss + 1);
        assert_eq!(toasts.active_now().len(), stale_items.len());
        assert_eq!(toasts.tracked_item_count(STALE_TASK_ID), 0);
        assert_eq!(toasts.tracked_item_count(fresh_task_id), stale_items.len());
    }

    /// `add_new_tracked_items` skips keys the toast already holds, so a later
    /// activity change only lands because `sync_running_toast` also refreshes
    /// activity on existing items.
    #[test]
    fn sync_running_toast_pushes_activity_changes_onto_existing_items() {
        let mut toasts = Toasts::<TestApp>::new();
        let mut items = [tracked_item(FIRST_ITEM)];

        let task_id = sync_running_toast(&mut toasts, None, RUNNING_TOAST_TITLE, &items)
            .expect("running items should create the toast");
        assert_eq!(
            item_activity(&toasts, task_id),
            TrackedItemActivity::Progressing
        );

        items[0].activity = TrackedItemActivity::Stalled;
        sync_running_toast(&mut toasts, Some(task_id), RUNNING_TOAST_TITLE, &items);
        assert_eq!(
            item_activity(&toasts, task_id),
            TrackedItemActivity::Stalled
        );

        items[0].activity = TrackedItemActivity::Progressing;
        sync_running_toast(&mut toasts, Some(task_id), RUNNING_TOAST_TITLE, &items);
        assert_eq!(
            item_activity(&toasts, task_id),
            TrackedItemActivity::Progressing
        );
    }

    fn item_activity(toasts: &Toasts<TestApp>, task_id: ToastTaskId) -> TrackedItemActivity {
        toasts
            .active_now()
            .iter()
            .find(|view| view.id() == ToastId(task_id.get()))
            .and_then(|view| view.tracked_items().first())
            .expect("the toast should carry its tracked item")
            .activity
    }

    fn sync_running_toast(
        toasts: &mut Toasts<TestApp>,
        slot: Option<ToastTaskId>,
        title: &str,
        items: &[TrackedItem],
    ) -> Option<ToastTaskId> {
        let slot = slot.map_or(RunningToastSlot::Vacant, RunningToastSlot::Assigned);
        match sync_running_task(
            toasts,
            slot,
            title,
            items,
            crate::toast_body_width(&crate::ToastSettings::default()),
        ) {
            RunningToastSlot::Vacant => None,
            RunningToastSlot::Assigned(id) => Some(id),
        }
    }

    fn tracked_item(label: &str) -> TrackedItem { TrackedItem::new(label, label) }
}

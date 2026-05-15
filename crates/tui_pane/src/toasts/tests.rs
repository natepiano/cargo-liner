use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;

use super::toast::ToastDismissal;
use super::toast::ToastPhase;
use super::*;
use crate::FocusedPane;
use crate::Framework;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::NoToastAction;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TestPaneId {
    Main,
}

struct TestApp {
    framework: Framework<Self>,
}

impl AppContext for TestApp {
    type AppPaneId = TestPaneId;
    type ToastAction = NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

fn toasts() -> Toasts<TestApp> { Toasts::new() }

#[test]
fn timed_toast_expires_at_timeout() {
    let mut toasts = toasts();
    let id = toasts.push_timed("done", "body", Duration::ZERO, 1);

    toasts.prune(Instant::now());

    assert!(!toasts.is_alive(id));
}

#[test]
fn persistent_toast_survives_prune() {
    let mut toasts = toasts();
    let id = toasts.push_persistent("error", "body", ToastStyle::Error, None, 1);

    toasts.prune(Instant::now() + Duration::from_secs(61));

    assert!(toasts.is_alive(id));
}

#[test]
fn dismiss_does_not_restart_an_already_exiting_animation() {
    let mut toasts = toasts();
    let task = toasts.start_task("scan", "running");
    let id = toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort())
        .id();

    // First dismiss: phase transitions to Exiting with the
    // initial started_at.
    assert!(toasts.dismiss(id));
    let first_started_at = match toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort())
        .phase
    {
        ToastPhase::Exiting { started_at } => started_at,
        ToastPhase::Visible => std::process::abort(),
    };

    // Spin a touch to make sure Instant::now() advances, then
    // dismiss again — the started_at must not reset.
    std::thread::sleep(Duration::from_millis(2));
    assert!(toasts.dismiss(id));
    let second_started_at = match toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort())
        .phase
    {
        ToastPhase::Exiting { started_at } => started_at,
        ToastPhase::Visible => std::process::abort(),
    };
    assert_eq!(first_started_at, second_started_at);
}

#[test]
fn user_dismissed_task_toast_is_not_revived_by_reactivate() {
    let mut toasts = toasts();
    let task = toasts.start_task("scan", "running");

    // User clicks [x].
    let toast_id = toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort())
        .id();
    assert!(toasts.dismiss(toast_id));

    // Tracker keeps reporting work; we ask reactivate_task to
    // re-show the toast. The user-dismissed flag suppresses
    // reactivation.
    assert_eq!(
        toasts.reactivate_task(task),
        super::ReactivateOutcome::DismissedByUser,
    );
    let toast = toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort());
    assert!(matches!(toast.phase, ToastPhase::Exiting { .. }));
    assert_eq!(toast.dismissal, ToastDismissal::ClosedByUser);
}

fn toasts_with_linger(linger_secs: f64) -> Toasts<TestApp> {
    let mut t = Toasts::<TestApp>::new();
    t.settings_mut().finished_task_visible =
        crate::ToastDuration::try_from_secs("test", linger_secs)
            .unwrap_or_else(|_| std::process::abort());
    t
}

#[test]
fn reactivate_task_revives_non_dismissed_finished_toast() {
    // Linger covers an item so finish_task records the toast
    // as still-finished rather than instantly-zero, which is
    // what `reactivate_task` is meant to recover from.
    let mut toasts = toasts_with_linger(30.0);
    let task = toasts.start_task("scan", "running");
    assert!(toasts.set_tracked_items(task, &[TrackedItem::new("a", "a")]));
    assert!(toasts.finish_task(task));

    assert_eq!(
        toasts.reactivate_task(task),
        super::ReactivateOutcome::Revived,
    );
    let toast = toasts
        .toast_for_task(task)
        .unwrap_or_else(|| std::process::abort());
    assert!(matches!(toast.phase, ToastPhase::Visible));
}

#[test]
fn reactivate_task_returns_not_found_for_unknown_task() {
    let mut toasts = toasts();
    let task = toasts.start_task("scan", "running");
    // No tracked items → finish_task uses Duration::ZERO.
    assert!(toasts.finish_task(task));
    let after_linger = Instant::now() + Duration::from_secs(2);
    toasts.prune(after_linger);

    let stale_task = ToastTaskId(99);
    assert_eq!(
        toasts.reactivate_task(stale_task),
        super::ReactivateOutcome::NotFound,
    );
}

#[test]
fn task_toast_lingers_after_finish_then_prunes() {
    let mut toasts = toasts_with_linger(1.0);
    let task = toasts.start_task("scan", "running");
    // Tracked item is what makes `finish_task` honor the
    // settings-driven linger.
    assert!(toasts.set_tracked_items(task, &[TrackedItem::new("a", "a")]));

    assert!(toasts.finish_task(task));
    toasts.prune(Instant::now());
    assert!(toasts.is_task_finished(task));

    let after_linger = Instant::now() + Duration::from_secs(2);
    toasts.prune(after_linger);
    toasts.prune(after_linger + Duration::from_secs(1));

    assert!(!toasts.is_task_finished(task));
}

#[test]
fn tracked_items_prune_after_linger() {
    let mut toasts = toasts_with_linger(0.0);
    let task = toasts.start_task("scan", "running");
    let item = TrackedItem::new("repo", "repo");
    assert!(toasts.set_tracked_items(task, &[item]));
    assert_eq!(toasts.tracked_item_count(task), 1);

    assert!(toasts.mark_tracked_item_completed(task, "repo"));
    toasts.prune_tracked_items(Instant::now() + Duration::from_secs(1));

    assert_eq!(toasts.tracked_item_count(task), 0);
}

#[test]
fn focused_toast_command_returns_action_payload() {
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum ToastAction {
        Open,
    }

    struct ActionApp {
        framework: Framework<Self>,
    }

    impl AppContext for ActionApp {
        type AppPaneId = TestPaneId;
        type ToastAction = ToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    let mut toasts = Toasts::<ActionApp>::new();
    let _ = toasts.push_with_action("open", "path", ToastAction::Open);

    let command = toasts.handle_key_command(&KeyBind::from(KeyCode::Enter));

    assert_eq!(
        command,
        (
            KeyOutcome::Consumed,
            ToastCommand::Activate(ToastAction::Open)
        )
    );
}

#[test]
fn new_toasts_do_not_move_existing_focus() {
    let mut toasts = toasts();
    let first = toasts.push("first", "body");
    let _second = toasts.push("second", "body");

    assert_eq!(toasts.focused_id(), Some(first));
}

#[test]
fn toasts_can_live_on_framework() {
    let mut app = TestApp {
        framework: Framework::new(FocusedPane::App(TestPaneId::Main)),
    };
    let _ = app.framework.toasts.push("hello", "body");

    assert!(app.framework.toasts.has_active());
}

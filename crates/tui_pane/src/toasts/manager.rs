use std::time::Duration;
use std::time::Instant;

use super::body::ToastBody;
use super::settings::ToastSettings;
use super::toast::Toast;
use super::toast::ToastLifetime;
use super::toast::ToastStyle;
use super::view::ToastHitbox;
use crate::AppContext;
use crate::Viewport;
use crate::constants::FRAME_POLL_MILLIS;

/// Result of handling a focused toast key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastCommand<A> {
    /// No toast action fired.
    None,
    /// The focused toast requested its action payload.
    Activate(A),
}

/// Earliest time when an active toast can next change its rendered content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastVisualDeadline {
    /// No active toast has a time-driven visual change scheduled.
    NoVisualChangeScheduled,
    /// A toast can change visually at this instant.
    At(Instant),
}

impl ToastVisualDeadline {
    pub(super) fn earlier(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoVisualChangeScheduled, deadline)
            | (deadline, Self::NoVisualChangeScheduled) => deadline,
            (Self::At(left), Self::At(right)) => Self::At(if left < right { left } else { right }),
        }
    }
}

pub(crate) struct ToastSpec<Ctx: AppContext> {
    pub(super) title:              String,
    pub(super) body:               ToastBody,
    pub(super) style:              ToastStyle,
    pub(super) lifetime:           ToastLifetime,
    pub(super) action:             Option<Ctx::ToastAction>,
    pub(super) min_interior_lines: usize,
    pub(super) item_linger:        Duration,
}

/// Outcome of [`Toasts::reactivate_task`].
///
/// Replaces a plain `bool` so callers can distinguish "no toast
/// for this task — create one" from "user dismissed this toast
/// — leave it alone." `bool` returns conflated those cases and
/// caused user-dismissed toasts to be re-created on the next
/// tracker poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactivateOutcome {
    /// No toast registered for this task id. Caller should
    /// create a fresh toast for the tracker.
    NotFound,
    /// Toast existed and was returned to
    /// running status. An in-flight entrance remains in progress; an exiting
    /// toast returns to `toast::ToastPhase::Static`.
    Revived,
    /// Toast existed but its dismissal is
    /// `toast::ToastDismissal::ClosedByUser`. Caller should neither
    /// touch the toast nor create a replacement — the user
    /// closed it, and the underlying tracker work continues
    /// without UI surface.
    DismissedByUser,
}

/// Framework-owned toast manager.
pub struct Toasts<Ctx: AppContext> {
    pub(super) next_id:  u64,
    pub(super) entries:  Vec<Toast<Ctx>>,
    /// Viewport used when focus is on the Toasts framework pane.
    pub viewport:        Viewport,
    pub(super) hits:     Vec<ToastHitbox>,
    pub(super) settings: ToastSettings,
}

impl<Ctx: AppContext> Default for Toasts<Ctx> {
    fn default() -> Self { Self::new() }
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Create an empty toast manager with default settings.
    #[must_use]
    pub fn new() -> Self { Self::with_settings(ToastSettings::default()) }

    /// Create an empty toast manager with explicit settings.
    #[must_use]
    pub fn with_settings(settings: ToastSettings) -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
            viewport: Viewport::default(),
            hits: Vec::new(),
            settings,
        }
    }

    /// Borrow the toast settings.
    #[must_use]
    pub const fn settings(&self) -> &ToastSettings { &self.settings }

    /// Mutably borrow the toast settings.
    pub const fn settings_mut(&mut self) -> &mut ToastSettings { &mut self.settings }

    /// Replace the toast settings.
    pub fn set_settings(&mut self, settings: ToastSettings) {
        self.settings = settings;
        let item_linger = self.settings.finished_task_visible.get();
        for toast in &mut self.entries {
            toast.refresh_entrance_phase(&self.settings);
            if matches!(toast.lifetime, ToastLifetime::Task { .. }) {
                toast.item_linger = item_linger;
            }
        }
    }

    /// Return the earliest instant when an active toast can next change what
    /// it renders.
    ///
    /// This scans the stored toasts without building [`super::ToastView`]s.
    /// Each toast combines its line-height, countdown, tracked-item, and
    /// lifetime deadlines.
    #[must_use]
    pub fn next_visual_change_deadline(&self, now: Instant) -> ToastVisualDeadline {
        if !self.settings.toasts_enabled() {
            return ToastVisualDeadline::NoVisualChangeScheduled;
        }
        let earliest_deadline = self.entries.iter().fold(
            ToastVisualDeadline::NoVisualChangeScheduled,
            |deadline, toast| {
                deadline.earlier(toast.next_visual_change_deadline(now, &self.settings))
            },
        );
        match earliest_deadline {
            ToastVisualDeadline::NoVisualChangeScheduled => earliest_deadline,
            ToastVisualDeadline::At(deadline) => ToastVisualDeadline::At(
                deadline.max(now + Duration::from_millis(FRAME_POLL_MILLIS)),
            ),
        }
    }

    pub(super) fn sync_viewport_len(&mut self) {
        let len = self.active_now().len();
        self.viewport.set_len(len);
        if len == 0 {
            self.viewport.set_pos(0);
        } else if self.viewport.pos() >= len {
            self.viewport.set_pos(len - 1);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unreachable,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use crossterm::event::KeyCode;
    use ratatui::style::Color;

    use super::*;
    use crate::ACTIVITY_SPINNER;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::KeyBind;
    use crate::KeyOutcome;
    use crate::NoToastAction;
    use crate::ToastDuration;
    use crate::ToastTaskId;
    use crate::TrackedItem;
    use crate::TrackedItemKey;
    use crate::toasts::toast::ToastDismissal;
    use crate::toasts::toast::ToastLifetime;
    use crate::toasts::toast::ToastPhase;
    use crate::toasts::toast::ToastTaskStatus;

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
    fn coarse_countdown_deadline_is_not_pulled_in_to_the_frame_poll_floor() {
        let visible = Duration::from_secs(5);
        let mut toasts = toasts();
        let id = toasts.push_timed("done", "body", visible, 1);
        let toast = toasts
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .expect("pushed toast should be stored");
        let created_at = toast.created_at;
        let ToastLifetime::Timed { timeout_at } = toast.lifetime else {
            unreachable!("push_timed should create a timed lifetime");
        };
        let first_countdown_change = created_at + Duration::from_secs(1);

        assert_eq!(
            toasts.next_visual_change_deadline(created_at),
            ToastVisualDeadline::At(first_countdown_change)
        );
        assert_eq!(timeout_at, created_at + visible);
    }

    #[test]
    fn multi_line_toast_first_repaint_follows_minimum_height_steps() {
        const MIN_INTERIOR_LINES: usize = 1;

        let settings = ToastSettings::default();
        let body = "x".repeat(crate::toast_body_width(&settings) * 4);
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        let id = toasts.push_timed(
            "Favorite not saved",
            body,
            Duration::from_secs(5),
            MIN_INTERIOR_LINES,
        );
        let toast = toasts
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .expect("pushed toast should be stored");
        let created_at = toast.created_at;
        let min_height = toasts.active_views(created_at)[0].min_height();
        let entrance_line = settings.animation.entrance_duration.get();
        let first_repaint_at = created_at + entrance_line.saturating_mul(u32::from(min_height));
        let before_first_repaint = first_repaint_at
            .checked_sub(Duration::from_nanos(1))
            .expect("first repaint should follow toast creation");

        assert_eq!(
            toasts.next_visual_change_deadline(created_at),
            ToastVisualDeadline::At(first_repaint_at)
        );
        assert_eq!(
            toasts.active_views(before_first_repaint)[0].desired_height(),
            min_height
        );
        assert_eq!(
            toasts.active_views(first_repaint_at)[0].desired_height(),
            min_height.saturating_add(1)
        );
    }

    #[test]
    fn new_tracked_items_refresh_a_task_toasts_entrance_schedule() {
        let settings = ToastSettings::default();
        let entrance_line = settings.animation.entrance_duration.get();
        let mut toasts = Toasts::<TestApp>::with_settings(settings);
        let task = toasts.start_task("scan", "running");
        let mut items = vec![
            TrackedItem::new("a", "a"),
            TrackedItem::new("b", "b"),
            TrackedItem::new("c", "c"),
            TrackedItem::new("d", "d"),
        ];
        for item in &mut items {
            item.started_at = None;
        }
        assert!(toasts.set_tracked_items(task, &items));
        let toast = toasts
            .toast_for_task(task)
            .expect("task toast should remain stored");
        let created_at = toast.created_at;
        let min_height = toasts.active_views(created_at)[0].min_height();
        let first_repaint_at = created_at + entrance_line.saturating_mul(u32::from(min_height));

        assert!(matches!(toast.phase, ToastPhase::Entering { .. }));
        assert_eq!(
            toasts.next_visual_change_deadline(created_at),
            ToastVisualDeadline::At(first_repaint_at)
        );
    }

    #[test]
    fn reactivate_task_preserves_an_in_flight_entrance() {
        let settings = ToastSettings::default();
        let mut toasts = Toasts::<TestApp>::with_settings(settings);
        let task = toasts.start_task("scan", "running");
        let mut items = vec![
            TrackedItem::new("a", "a"),
            TrackedItem::new("b", "b"),
            TrackedItem::new("c", "c"),
            TrackedItem::new("d", "d"),
        ];
        for item in &mut items {
            item.started_at = None;
        }
        assert!(toasts.set_tracked_items(task, &items));
        let toast = toasts
            .toast_for_task(task)
            .expect("task toast should remain stored");
        let created_at = toast.created_at;
        let ToastPhase::Entering { starts_at, .. } = toast.phase else {
            unreachable!("multi-row task toast should be entering");
        };

        assert_eq!(toasts.reactivate_task(task), ReactivateOutcome::Revived);

        let toast = toasts
            .toast_for_task(task)
            .expect("reactivated task toast should remain stored");
        assert!(matches!(toast.phase, ToastPhase::Entering { .. }));
        assert_eq!(
            toasts.next_visual_change_deadline(created_at),
            ToastVisualDeadline::At(starts_at)
        );
    }

    #[test]
    fn running_task_schedules_the_next_spinner_frame() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");
        let now = Instant::now();
        let started_at = now
            .checked_sub(Duration::from_secs(20))
            .expect("test instant should support a twenty-second offset");
        let mut item = TrackedItem::new("repo", "repo");
        item.started_at = Some(started_at);
        assert!(toasts.set_tracked_items(task, &[item]));
        let elapsed = now.saturating_duration_since(started_at);
        let expected = started_at + ACTIVITY_SPINNER.next_frame_boundary(elapsed);

        assert_eq!(
            toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(expected)
        );
    }

    #[test]
    fn running_task_millisecond_deadline_respects_the_frame_poll_floor() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");
        let now = Instant::now();
        let started_at = now
            .checked_sub(Duration::from_secs(5))
            .expect("test instant should support a five-second offset");
        let mut item = TrackedItem::new("repo", "repo");
        item.started_at = Some(started_at);
        assert!(toasts.set_tracked_items(task, &[item]));

        let ToastVisualDeadline::At(deadline) = toasts.next_visual_change_deadline(now) else {
            unreachable!("a running tracked item should schedule a visual change");
        };
        assert!(deadline >= now + Duration::from_millis(FRAME_POLL_MILLIS));
    }

    #[test]
    fn finished_task_without_items_schedules_linger_fade_before_countdown() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");
        let toast = toasts
            .toast_for_task_mut(task)
            .expect("task toast should be stored");
        let finished_at = toast.created_at;
        let linger = Duration::from_secs(5);
        toast.lifetime = ToastLifetime::Task {
            task_id: task,
            status:  ToastTaskStatus::Finished {
                finished_at,
                linger,
            },
        };
        toast.phase = ToastPhase::Static;
        assert!(toast.tracked_items.is_empty());
        let now = finished_at + Duration::from_secs(2);
        let next_countdown_boundary = finished_at + Duration::from_secs(3);

        let ToastVisualDeadline::At(deadline) = toasts.next_visual_change_deadline(now) else {
            unreachable!("a lingering finished toast should schedule its next fade level");
        };
        assert!(deadline > now);
        assert!(deadline < next_countdown_boundary);
    }

    #[test]
    fn completed_item_end_precedes_the_task_countdown_boundary() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("scan", "running");
        let completed_at = toasts
            .toast_for_task(task)
            .expect("task toast should be stored")
            .created_at;
        let mut first = TrackedItem::new("first", "first");
        first.started_at = None;
        first.completed_at = Some(completed_at);
        let mut second = TrackedItem::new("second", "second");
        second.started_at = None;
        second.completed_at = Some(completed_at + Duration::from_millis(2_400));
        assert!(toasts.set_tracked_items(task, &[first, second]));
        let now = completed_at + Duration::from_millis(4_990);
        let first_item_ends_at = completed_at + Duration::from_secs(5);

        assert_eq!(
            toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(first_item_ends_at)
        );
        assert!(matches!(
            toasts.next_visual_change_deadline(first_item_ends_at),
            ToastVisualDeadline::At(deadline) if deadline > first_item_ends_at
        ));
    }

    #[test]
    fn settings_reload_refreshes_entrance_height_and_deadline() {
        const MIN_INTERIOR_LINES: usize = 1;

        let original_settings = ToastSettings::default();
        let body = "x".repeat(crate::toast_body_width(&original_settings) * 4);
        let mut toasts = Toasts::<TestApp>::with_settings(original_settings);
        let id = toasts.push_timed(
            "Favorite not saved",
            body,
            Duration::from_secs(5),
            MIN_INTERIOR_LINES,
        );
        let created_at = toasts
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .expect("pushed toast should be stored")
            .created_at;
        let mut updated_settings = ToastSettings::default();
        updated_settings.animation.entrance_duration = ToastDuration::try_from_secs("test", 0.25)
            .expect("test entrance duration should be valid");

        toasts.set_settings(updated_settings.clone());

        let min_height = toasts.active_views(created_at)[0].min_height();
        let first_repaint_at = created_at
            + updated_settings
                .animation
                .entrance_duration
                .get()
                .saturating_mul(u32::from(min_height));
        let before_first_repaint = first_repaint_at
            .checked_sub(Duration::from_nanos(1))
            .expect("first repaint should follow toast creation");
        assert_eq!(
            toasts.next_visual_change_deadline(created_at),
            ToastVisualDeadline::At(first_repaint_at)
        );
        assert_eq!(
            toasts.active_views(before_first_repaint)[0].desired_height(),
            min_height
        );
        assert_eq!(
            toasts.active_views(first_repaint_at)[0].desired_height(),
            min_height.saturating_add(1)
        );
    }

    #[test]
    fn settings_reload_refreshes_tracked_item_removal_deadline() {
        let mut toasts = toasts_with_linger(10.0);
        let task = toasts.start_task("scan", "running");
        let completed_at = toasts
            .toast_for_task(task)
            .expect("task toast should be stored")
            .created_at;
        let mut item = TrackedItem::new("repo", "repo");
        item.started_at = None;
        item.completed_at = Some(completed_at);
        assert!(toasts.set_tracked_items(task, &[item]));

        let updated_linger = Duration::from_secs(5);
        let mut updated_settings = toasts.settings().clone();
        updated_settings.finished_task_visible =
            ToastDuration::try_from_secs("test", updated_linger.as_secs_f64())
                .expect("test linger duration should be valid");
        toasts.set_settings(updated_settings);

        let toast = toasts
            .toast_for_task_mut(task)
            .expect("task toast should remain stored");
        assert_eq!(toast.item_linger, updated_linger);
        toast.lifetime = ToastLifetime::Task {
            task_id: task,
            status:  ToastTaskStatus::Running,
        };
        let removal_at = completed_at + updated_linger;
        let now = removal_at
            .checked_sub(Duration::from_millis(FRAME_POLL_MILLIS + 2))
            .expect("removal should follow completion");

        assert_eq!(
            toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(removal_at)
        );
    }

    #[test]
    fn exit_deadlines_follow_existing_line_height_boundaries() {
        const MIN_INTERIOR_LINES: usize = 1;

        let settings = ToastSettings::default();
        let exit_line = settings.animation.exit_duration.get();
        let body = "x".repeat(crate::toast_body_width(&settings) * 3);
        let mut toasts = Toasts::<TestApp>::with_settings(settings);
        let id = toasts.push_timed(
            "Favorite not saved",
            body,
            Duration::from_secs(2),
            MIN_INTERIOR_LINES,
        );
        let toast = toasts
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .expect("pushed toast should be stored");
        let ToastLifetime::Timed { timeout_at } = toast.lifetime else {
            unreachable!("push_timed should create a timed lifetime");
        };

        toasts.prune(timeout_at);

        let target_height = toasts.active_views(timeout_at)[0].desired_height();
        let first_exit_repaint_at = timeout_at + exit_line;
        let exit_ends_at = timeout_at + exit_line.saturating_mul(u32::from(target_height));
        let before_exit_end = exit_ends_at
            .checked_sub(Duration::from_millis(FRAME_POLL_MILLIS.saturating_mul(2)))
            .expect("exit end should follow expiry");

        assert_eq!(
            toasts.next_visual_change_deadline(timeout_at),
            ToastVisualDeadline::At(first_exit_repaint_at)
        );
        assert_eq!(
            toasts.active_views(first_exit_repaint_at)[0].desired_height(),
            target_height.saturating_sub(1)
        );
        assert_eq!(
            toasts.next_visual_change_deadline(before_exit_end),
            ToastVisualDeadline::At(exit_ends_at)
        );
        assert!(toasts.active_views(exit_ends_at).is_empty());
        assert_eq!(
            toasts.next_visual_change_deadline(exit_ends_at),
            ToastVisualDeadline::NoVisualChangeScheduled
        );
    }

    #[test]
    fn persistent_toast_survives_prune() {
        let mut toasts = toasts();
        let id = toasts.push_persistent("error", "body", ToastStyle::Error, None, 1);

        toasts.prune(Instant::now() + Duration::from_secs(61));

        assert!(toasts.is_alive(id));
    }

    #[test]
    fn colored_countdown_uses_timed_lifetime_not_task_linger() {
        let mut toasts = toasts();
        let id = toasts.push_colored_persistent(
            "Startup",
            vec!["Disk usage 100%".to_string()],
            vec![Color::Green],
        );

        assert!(
            toasts.update_colored(id, vec!["Disk usage 100%".to_string()], vec![Color::Green],)
        );
        assert!(toasts.start_colored_countdown(id, Duration::from_secs(5)));

        let view = toasts
            .active_now()
            .into_iter()
            .find(|toast| toast.title() == "Startup")
            .expect("startup colored toast should be active");
        assert_eq!(view.linger_progress(), None);
        assert!(view.remaining_secs().is_some());
        assert_eq!(view.body_line_colors(), Some(&[Color::Green][..]));
    }

    #[test]
    fn dismiss_does_not_restart_an_already_exiting_animation() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");
        let id = toasts
            .toast_for_task(task)
            .expect("start_task should create a task toast")
            .id();

        // First dismiss: phase transitions to Exiting with the
        // initial started_at.
        assert!(toasts.dismiss(id));
        let first_started_at = match toasts
            .toast_for_task(task)
            .expect("dismissed task toast should still be tracked")
            .phase
        {
            ToastPhase::Exiting { started_at } => started_at,
            ToastPhase::Entering { .. } | ToastPhase::Static => {
                unreachable!("dismissed toast should enter Exiting phase");
            },
        };

        // Spin a touch to make sure Instant::now() advances, then
        // dismiss again — the started_at must not reset.
        std::thread::sleep(Duration::from_millis(2));
        assert!(toasts.dismiss(id));
        let second_started_at = match toasts
            .toast_for_task(task)
            .expect("dismissed task toast should still be tracked")
            .phase
        {
            ToastPhase::Exiting { started_at } => started_at,
            ToastPhase::Entering { .. } | ToastPhase::Static => {
                unreachable!("second dismiss should keep toast Exiting");
            },
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
            .expect("start_task should create a task toast")
            .id();
        assert!(toasts.dismiss(toast_id));

        // Tracker keeps reporting work; we ask reactivate_task to
        // re-show the toast. The user-dismissed flag suppresses
        // reactivation.
        assert_eq!(
            toasts.reactivate_task(task),
            super::super::ReactivateOutcome::DismissedByUser,
        );
        let toast = toasts
            .toast_for_task(task)
            .expect("dismissed task toast should remain tracked");
        assert!(matches!(toast.phase, ToastPhase::Exiting { .. }));
        assert_eq!(toast.dismissal, ToastDismissal::ClosedByUser);
    }

    fn toasts_with_linger(linger_secs: f64) -> Toasts<TestApp> {
        let mut t = Toasts::<TestApp>::new();
        t.settings_mut().finished_task_visible =
            crate::ToastDuration::try_from_secs("test", linger_secs)
                .expect("test linger duration should be valid");
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
            super::super::ReactivateOutcome::Revived,
        );
        let toast = toasts
            .toast_for_task(task)
            .expect("revived task toast should remain tracked");
        assert!(matches!(toast.phase, ToastPhase::Static));
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
            super::super::ReactivateOutcome::NotFound,
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
        let _ = toasts.push("second", "body");

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

    #[test]
    fn marking_last_item_completed_auto_finishes_task_toast() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(
            task,
            &[TrackedItem::new("a", "a"), TrackedItem::new("b", "b")],
        ));

        assert!(toasts.mark_tracked_item_completed(task, "a"));
        assert!(
            !toasts.is_task_finished(task),
            "auto-finish must not fire while any item is incomplete",
        );

        assert!(toasts.mark_tracked_item_completed(task, "b"));
        assert!(
            toasts.is_task_finished(task),
            "marking the final item completed must transition the toast to Finished",
        );
    }

    #[test]
    fn auto_finish_does_not_fire_with_pending_items() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(
            task,
            &[
                TrackedItem::new("a", "a"),
                TrackedItem::new("b", "b"),
                TrackedItem::new("c", "c"),
            ],
        ));

        assert!(toasts.mark_tracked_item_completed(task, "a"));
        assert!(toasts.mark_tracked_item_completed(task, "b"));
        assert!(!toasts.is_task_finished(task));
    }

    #[test]
    fn auto_finish_does_not_fire_on_zero_item_task_toast() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("empty", "body");

        // No items were ever added. Auto-finish must not fire on its
        // own — an empty task toast stays Running until the embedding
        // calls `finish_task` explicitly.
        assert!(!toasts.is_task_finished(task));

        assert!(toasts.finish_task(task));
        assert!(toasts.is_task_finished(task));
    }

    #[test]
    fn finish_task_anchors_finished_at_to_last_item_completion() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(task, &[TrackedItem::new("only", "only")]));

        assert!(toasts.mark_tracked_item_completed(task, "only"));
        let toast = toasts
            .toast_for_task(task)
            .expect("completed task toast should remain tracked");
        let ToastLifetime::Task {
            status:
                ToastTaskStatus::Finished {
                    finished_at: original_finished_at,
                    ..
                },
            ..
        } = toast.lifetime
        else {
            unreachable!("completed task toast should use task lifetime");
        };

        std::thread::sleep(Duration::from_millis(5));
        // Calling finish_task again must not bump `finished_at` forward —
        // the anchor is `max(item.completed_at)`, which hasn't moved
        // because no item was re-marked. The countdown stays stable.
        assert!(toasts.finish_task(task));

        let toast = toasts
            .toast_for_task(task)
            .expect("completed task toast should remain tracked");
        let ToastLifetime::Task {
            status:
                ToastTaskStatus::Finished {
                    finished_at: later_finished_at,
                    ..
                },
            ..
        } = toast.lifetime
        else {
            unreachable!("completed task toast should use task lifetime");
        };
        assert_eq!(original_finished_at, later_finished_at);
    }

    #[test]
    fn adding_incomplete_item_reverts_finished_toast_to_running() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(task, &[TrackedItem::new("first", "first")]));

        // Mark the only item completed → toast auto-finishes.
        assert!(toasts.mark_tracked_item_completed(task, "first"));
        assert!(toasts.is_task_finished(task));

        // A new incomplete item arrives after auto-finish (e.g. a
        // tracker queues a late phase). The toast must revert to
        // Running so the countdown re-anchors when the new item
        // eventually completes.
        assert!(toasts.add_new_tracked_items(task, &[TrackedItem::new("late", "late")]));
        assert!(!toasts.is_task_finished(task));

        // When the late item finally completes, auto-finish re-fires.
        assert!(toasts.mark_tracked_item_completed(task, "late"));
        assert!(toasts.is_task_finished(task));
    }

    #[test]
    fn late_completion_extends_finished_at_past_original_anchor() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(task, &[TrackedItem::new("first", "first")]));

        assert!(toasts.mark_tracked_item_completed(task, "first"));
        let toast = toasts
            .toast_for_task(task)
            .expect("first completed task toast should remain tracked");
        let ToastLifetime::Task {
            status:
                ToastTaskStatus::Finished {
                    finished_at: anchor_after_first,
                    ..
                },
            ..
        } = toast.lifetime
        else {
            unreachable!("first completed task toast should use task lifetime");
        };

        // Add a second incomplete item, sleep, mark it completed.
        assert!(toasts.add_new_tracked_items(task, &[TrackedItem::new("late", "late")]));
        std::thread::sleep(Duration::from_millis(5));
        assert!(toasts.mark_tracked_item_completed(task, "late"));

        let toast = toasts
            .toast_for_task(task)
            .expect("late completed task toast should remain tracked");
        let ToastLifetime::Task {
            status:
                ToastTaskStatus::Finished {
                    finished_at: anchor_after_late,
                    ..
                },
            ..
        } = toast.lifetime
        else {
            unreachable!("late completed task toast should use task lifetime");
        };
        assert!(
            anchor_after_late > anchor_after_first,
            "finished_at must move forward when a later item completes",
        );
    }

    #[test]
    fn task_toast_skips_exit_animation_after_countdown() {
        let mut toasts = toasts_with_linger(0.0);
        let task = toasts.start_task("phase", "body");
        assert!(toasts.set_tracked_items(task, &[TrackedItem::new("only", "only")]));
        assert!(toasts.mark_tracked_item_completed(task, "only"));
        // finished_at + linger(=0) <= now, so should_exit is true on
        // the next prune. Task toasts skip the Exiting render phase
        // entirely: the toast is removed in the same prune pass.
        toasts.prune(Instant::now());
        assert!(!toasts.is_task_finished(task));
        assert!(toasts.toast_for_task(task).is_none());
    }

    #[test]
    fn restarting_a_completed_item_reverts_finished_toast_to_running() {
        let mut toasts = toasts_with_linger(5.0);
        let task = toasts.start_task("phase", "body");
        let key = TrackedItemKey::new("a");
        assert!(toasts.set_tracked_items(task, &[TrackedItem::new("a", key.clone())]));
        assert!(toasts.mark_tracked_item_completed(task, "a"));
        assert!(toasts.is_task_finished(task));

        // Restarting clears the item's `completed_at` — the toast
        // reverts to Running until the item completes again.
        assert!(toasts.restart_tracked_item(task, &key, Instant::now()));
        assert!(!toasts.is_task_finished(task));
    }
}

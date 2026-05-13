use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;

use super::Toast;
use super::ToastBody;
use super::ToastHitbox;
use super::ToastId;
use super::ToastLifetime;
use super::ToastPhase;
use super::ToastStyle;
use super::ToastTaskId;
use super::ToastTaskStatus;
use super::ToastView;
use super::TrackedItem;
use super::TrackedItemKey;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::CycleDirection;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::ListNavigation;
use crate::Mode;
use crate::ToastSettings;
use crate::Viewport;
use crate::keymap::Action;
use crate::panes::ToastsAction;

/// Result of handling a focused toast key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastCommand<A> {
    /// No toast action fired.
    None,
    /// The focused toast requested its action payload.
    Activate(A),
}

struct ToastSpec<Ctx: AppContext> {
    title:              String,
    body:               ToastBody,
    style:              ToastStyle,
    lifetime:           ToastLifetime,
    action:             Option<Ctx::ToastAction>,
    min_interior_lines: usize,
    item_linger:        Duration,
}

/// Framework-owned toast manager.
pub struct Toasts<Ctx: AppContext> {
    next_id:      u64,
    entries:      Vec<Toast<Ctx>>,
    /// Viewport used when focus is on the Toasts framework pane.
    pub viewport: Viewport,
    hits:         Vec<ToastHitbox>,
}

impl<Ctx: AppContext> Default for Toasts<Ctx> {
    fn default() -> Self { Self::new() }
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Create an empty toast manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id:  1,
            entries:  Vec::new(),
            viewport: Viewport::default(),
            hits:     Vec::new(),
        }
    }

    /// Push a persistent informational toast.
    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId {
        self.push_persistent_styled(title, body, ToastStyle::Normal, None, 1)
    }

    /// Push a persistent toast with an explicit style.
    pub fn push_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
    ) -> ToastId {
        self.push_persistent_styled(title, body, style, None, 1)
    }

    /// Push a persistent toast with an action payload.
    pub fn push_with_action(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        action: Ctx::ToastAction,
    ) -> ToastId {
        self.push_persistent_styled(title, body, ToastStyle::Normal, Some(action), 1)
    }

    /// Push a timed informational toast.
    pub fn push_timed(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_timed_styled(title, body, timeout, min_interior_lines, ToastStyle::Normal)
    }

    /// Push a timed toast with an explicit style.
    pub fn push_timed_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
        min_interior_lines: usize,
        style: ToastStyle,
    ) -> ToastId {
        let now = Instant::now();
        self.push_entry(
            ToastSpec {
                title:    title.into(),
                body:     ToastBody::from(body.into()),
                style,
                lifetime: ToastLifetime::Timed {
                    timeout_at: now + timeout,
                },
                action:   None,
                min_interior_lines,
                item_linger: Duration::ZERO,
            },
            now,
        )
    }

    /// Push a task-backed toast and return its task identifier.
    pub fn push_task(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        min_interior_lines: usize,
    ) -> ToastTaskId {
        let id = ToastTaskId(self.next_id);
        let now = Instant::now();
        self.push_entry(
            ToastSpec {
                title:    title.into(),
                body:     ToastBody::from(body.into()),
                style:    ToastStyle::Normal,
                lifetime: ToastLifetime::Task {
                    task_id: id,
                    status:  ToastTaskStatus::Running,
                },
                action:   None,
                min_interior_lines,
                item_linger: Duration::ZERO,
            },
            now,
        );
        id
    }

    /// Start a task-backed toast using the default body height.
    pub fn start_task(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastTaskId {
        self.push_task(title, body, 1)
    }

    /// Push a persistent toast with explicit style, action, and body height.
    pub fn push_persistent(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action: Option<Ctx::ToastAction>,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_entry(
            ToastSpec {
                title:    title.into(),
                body:     ToastBody::from(body.into()),
                style,
                lifetime: ToastLifetime::Persistent,
                action,
                min_interior_lines,
                item_linger: Duration::ZERO,
            },
            Instant::now(),
        )
    }

    /// Push a persistent toast with explicit style, action, and body height.
    pub fn push_persistent_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action: Option<Ctx::ToastAction>,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_persistent(title, body, style, action, min_interior_lines)
    }

    /// Start dismissing the toast with `id`.
    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let now = Instant::now();
        let Some(toast) = self.entries.iter_mut().find(|toast| toast.id == id) else {
            return false;
        };
        toast.phase = ToastPhase::Exiting { started_at: now };
        true
    }

    /// Start dismissing the currently focused toast.
    pub fn dismiss_focused(&mut self) -> bool {
        self.focused_toast_id().is_some_and(|id| self.dismiss(id))
    }

    /// Return the focused toast identifier.
    #[must_use]
    pub fn focused_id(&self) -> Option<ToastId> { self.focused_toast_id() }

    /// Return the focused toast identifier.
    pub fn focused_toast_id(&self) -> Option<ToastId> {
        self.active_now()
            .get(self.viewport.pos())
            .map(ToastView::id)
    }

    /// Return whether any live, non-exiting toast is active.
    #[must_use]
    pub fn has_active(&self) -> bool {
        let now = Instant::now();
        self.entries.iter().any(|toast| toast.is_live(now))
    }

    /// Return all stored toast entries.
    #[must_use]
    pub fn active(&self) -> &[Toast<Ctx>] { &self.entries }

    /// Return renderable toast views using default timing settings.
    #[must_use]
    pub fn active_now(&self) -> Vec<ToastView> {
        self.active_views(Instant::now(), &ToastSettings::default())
    }

    /// Return renderable toast views at `now`.
    #[must_use]
    pub fn active_views(&self, now: Instant, settings: &ToastSettings) -> Vec<ToastView> {
        self.entries
            .iter()
            .filter(|toast| toast.is_renderable(now, settings))
            .map(|toast| toast.view(now, settings))
            .collect()
    }

    /// Mark a task toast as finished.
    pub fn finish_task(&mut self, task_id: ToastTaskId, linger: Duration) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let now = Instant::now();
        toast.lifetime = ToastLifetime::Task {
            task_id,
            status: ToastTaskStatus::Finished {
                finished_at: now,
                linger,
            },
        };
        true
    }

    /// Mark a finished task toast as running again.
    pub fn reactivate_task(&mut self, task_id: ToastTaskId) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.lifetime = ToastLifetime::Task {
            task_id,
            status: ToastTaskStatus::Running,
        };
        toast.phase = ToastPhase::Visible;
        true
    }

    /// Replace the body text for a task toast.
    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.body = ToastBody::from(body.into());
        true
    }

    /// Replace the tracked-item list for a task toast.
    pub fn set_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        items: &[TrackedItem],
        linger: Duration,
    ) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.tracked_items = items.to_vec();
        toast.item_linger = linger;
        true
    }

    /// Return whether the toast is currently live.
    #[must_use]
    pub fn is_alive(&self, id: ToastId) -> bool {
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
        changed
    }

    /// Add tracked items whose keys are not already present.
    pub fn add_new_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        items: &[TrackedItem],
        item_linger: Duration,
    ) -> bool {
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
        changed
    }

    /// Restart one tracked item by key.
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
        true
    }

    /// Mark one tracked item completed by string key.
    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) -> bool {
        self.mark_item_completed(task_id, &TrackedItemKey::new(key))
    }

    /// Drop completed tracked items whose linger duration has elapsed.
    pub fn prune_tracked_items(&mut self, now: Instant, linger: Duration) {
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
        let settings = ToastSettings::default();
        for toast in &mut self.entries {
            if matches!(toast.phase, ToastPhase::Visible) && toast.should_exit(now) {
                toast.phase = ToastPhase::Exiting { started_at: now };
            }
        }
        self.entries
            .retain(|toast| toast.is_renderable(now, &settings));
        self.sync_viewport_len();
    }

    /// Move the toast viewport to the first toast when one exists.
    pub fn reset_to_first(&mut self) {
        if self.has_active() {
            self.viewport.set_pos(0);
        }
    }

    /// Move the toast viewport to the last toast when one exists.
    pub fn reset_to_last(&mut self) {
        let len = self.active_now().len();
        if len > 0 {
            self.viewport.set_pos(len - 1);
        }
    }

    /// Apply list navigation while focus is on the Toasts pane.
    pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome {
        let len = self.active_now().len();
        if len == 0 {
            return KeyOutcome::Unhandled;
        }
        self.viewport.set_len(len);
        match nav {
            ListNavigation::Up => self.viewport.up(),
            ListNavigation::Down => self.viewport.down(),
            ListNavigation::Home => self.viewport.home(),
            ListNavigation::End => self.viewport.end(),
        }
        KeyOutcome::Consumed
    }

    /// Consume a focus-cycle step as toast scrolling when possible.
    pub fn try_consume_cycle_step(&mut self, direction: CycleDirection) -> bool {
        let len = self.active_now().len();
        self.viewport.set_len(len);
        match direction {
            CycleDirection::Next if self.viewport.pos() + 1 < len => {
                self.viewport.down();
                true
            },
            CycleDirection::Prev if self.viewport.pos() > 0 => {
                self.viewport.up();
                true
            },
            CycleDirection::Next | CycleDirection::Prev => false,
        }
    }

    /// Handle a key and return both key outcome and toast action command.
    pub fn handle_key_command(
        &mut self,
        bind: &KeyBind,
    ) -> (KeyOutcome, ToastCommand<Ctx::ToastAction>) {
        let scope = Self::defaults().into_scope_map();
        if scope.action_for(bind) != Some(ToastsAction::Activate) {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        }

        let Some(id) = self.focused_toast_id() else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        let Some(action) = self
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .and_then(|toast| toast.action.clone())
        else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        (KeyOutcome::Consumed, ToastCommand::Activate(action))
    }

    /// Handle a key and return only the key outcome.
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome { self.handle_key_command(bind).0 }

    /// Return the Toasts pane mode.
    pub const fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Return default Toasts-pane bindings.
    #[must_use]
    pub fn defaults() -> Bindings<ToastsAction> {
        crate::bindings! {
            KeyCode::Enter => ToastsAction::Activate,
        }
    }

    /// Return status-bar slots for the Toasts pane.
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|action| (BarRegion::PaneAction, BarSlot::Single(action)))
            .collect()
    }

    /// Return hitboxes from the last toast render pass.
    #[must_use]
    pub fn hits(&self) -> &[ToastHitbox] { &self.hits }

    /// Replace hitboxes from the latest toast render pass.
    pub fn set_hits(&mut self, hits: Vec<ToastHitbox>) { self.hits = hits; }

    fn push_entry(&mut self, spec: ToastSpec<Ctx>, now: Instant) -> ToastId {
        let id = ToastId(self.next_id);
        self.next_id += 1;
        self.entries.push(Toast {
            id,
            title: spec.title,
            body: spec.body,
            style: spec.style,
            lifetime: spec.lifetime,
            phase: ToastPhase::Visible,
            action: spec.action,
            tracked_items: Vec::new(),
            created_at: now,
            min_interior_lines: spec.min_interior_lines,
            item_linger: spec.item_linger,
        });
        self.sync_viewport_len();
        id
    }

    fn toast_for_task(&self, task_id: ToastTaskId) -> Option<&Toast<Ctx>> {
        self.entries
            .iter()
            .find(|toast| toast.task_id() == Some(task_id))
    }

    fn toast_for_task_mut(&mut self, task_id: ToastTaskId) -> Option<&mut Toast<Ctx>> {
        self.entries
            .iter_mut()
            .find(|toast| toast.task_id() == Some(task_id))
    }

    fn sync_viewport_len(&mut self) {
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
    clippy::unwrap_used,
    reason = "tests should fail loudly on unexpected values"
)]
mod tests {
    use super::*;
    use crate::FocusedPane;
    use crate::Framework;
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
    fn task_toast_lingers_after_finish_then_prunes() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");

        assert!(toasts.finish_task(task, Duration::from_secs(1)));
        toasts.prune(Instant::now());
        assert!(toasts.is_task_finished(task));

        let after_linger = Instant::now() + Duration::from_secs(2);
        toasts.prune(after_linger);
        toasts.prune(after_linger + Duration::from_secs(1));

        assert!(!toasts.is_task_finished(task));
    }

    #[test]
    fn tracked_items_prune_after_linger() {
        let mut toasts = toasts();
        let task = toasts.start_task("scan", "running");
        let item = TrackedItem::new("repo", "repo");
        assert!(toasts.set_tracked_items(task, &[item], Duration::from_millis(10)));
        assert_eq!(toasts.tracked_item_count(task), 1);

        assert!(toasts.mark_tracked_item_completed(task, "repo"));
        toasts.prune_tracked_items(Instant::now() + Duration::from_secs(1), Duration::ZERO);

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
}

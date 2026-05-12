#![expect(
    missing_docs,
    reason = "Phase 26 migrates cargo-port's toast API into tui_pane; detailed docs land after the API settles"
)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::too_many_arguments,
    reason = "Phase 26 preserves the migrated toast-manager surface before public API polish"
)]

use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastCommand<A> {
    None,
    Activate(A),
}

pub fn toast_body_width(settings: &ToastSettings) -> usize {
    usize::from(settings.width.get().saturating_sub(2))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastId(pub u64);

impl ToastId {
    pub const fn get(self) -> u64 { self.0 }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastTaskId(pub u64);

impl ToastTaskId {
    pub const fn get(self) -> u64 { self.0 }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastStyle {
    Normal,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastBody {
    Text(String),
    Lines(Vec<String>),
}

impl ToastBody {
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Lines(lines) => lines.join("\n"),
        }
    }

    fn wrapped_line_count(&self, width: usize) -> usize {
        let width = width.max(1);
        self.as_text()
            .lines()
            .map(|line| (line.chars().count().max(1).saturating_sub(1) / width) + 1)
            .sum::<usize>()
            .max(1)
    }
}

impl From<String> for ToastBody {
    fn from(value: String) -> Self {
        if value.contains('\n') {
            Self::Lines(value.lines().map(ToOwned::to_owned).collect())
        } else {
            Self::Text(value)
        }
    }
}

impl From<&str> for ToastBody {
    fn from(value: &str) -> Self { Self::from(value.to_owned()) }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TrackedItemKey(String);

impl TrackedItemKey {
    pub fn new(value: impl Into<String>) -> Self { Self(value.into()) }

    pub fn as_str(&self) -> &str { &self.0 }
}

impl From<String> for TrackedItemKey {
    fn from(value: String) -> Self { Self(value) }
}

impl From<&str> for TrackedItemKey {
    fn from(value: &str) -> Self { Self(value.to_owned()) }
}

impl fmt::Display for TrackedItemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

#[derive(Clone, Debug)]
pub struct TrackedItem {
    pub label:        String,
    pub key:          TrackedItemKey,
    pub started_at:   Option<Instant>,
    pub completed_at: Option<Instant>,
}

impl TrackedItem {
    pub fn new(label: impl Into<String>, key: impl Into<TrackedItemKey>) -> Self {
        Self {
            label:        label.into(),
            key:          key.into(),
            started_at:   Some(Instant::now()),
            completed_at: None,
        }
    }

    pub fn label(&self) -> &str { &self.label }

    pub fn key(&self) -> &TrackedItemKey { &self.key }

    pub fn completed_at(&self) -> Option<Instant> { self.completed_at }

    pub fn mark_completed(&mut self, now: Instant) { self.completed_at = Some(now); }
}

#[derive(Clone, Copy, Debug)]
pub enum ToastLifetime {
    Timed {
        timeout_at: Instant,
    },
    Task {
        task_id: ToastTaskId,
        status:  ToastTaskStatus,
    },
    Persistent,
}

#[derive(Clone, Copy, Debug)]
pub enum ToastTaskStatus {
    Running,
    Finished {
        finished_at: Instant,
        linger:      Duration,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum ToastPhase {
    Visible,
    Exiting { started_at: Instant },
}

#[derive(Clone, Debug)]
pub struct Toast<Ctx: AppContext> {
    id:                 ToastId,
    title:              String,
    body:               ToastBody,
    style:              ToastStyle,
    lifetime:           ToastLifetime,
    phase:              ToastPhase,
    action:             Option<Ctx::ToastAction>,
    tracked_items:      Vec<TrackedItem>,
    created_at:         Instant,
    min_interior_lines: usize,
    item_linger:        Duration,
}

impl<Ctx: AppContext> Toast<Ctx> {
    pub fn id(&self) -> ToastId { self.id }

    pub fn title(&self) -> &str { &self.title }

    pub fn body(&self) -> &ToastBody { &self.body }

    pub fn body_text(&self) -> String { self.body.as_text() }

    pub fn style(&self) -> ToastStyle { self.style }

    pub fn action(&self) -> Option<&Ctx::ToastAction> { self.action.as_ref() }
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
        framework:    Framework<Self>,
        app_settings: (),
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type AppSettings = ();
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
        fn app_settings(&self) -> &Self::AppSettings { &self.app_settings }
        fn app_settings_mut(&mut self) -> &mut Self::AppSettings { &mut self.app_settings }
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

        assert!(toasts.mark_item_completed(task, "repo"));
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
            framework:    Framework<Self>,
            app_settings: (),
        }

        impl AppContext for ActionApp {
            type AppPaneId = TestPaneId;
            type AppSettings = ();
            type ToastAction = ToastAction;

            fn framework(&self) -> &Framework<Self> { &self.framework }
            fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
            fn app_settings(&self) -> &Self::AppSettings { &self.app_settings }
            fn app_settings_mut(&mut self) -> &mut Self::AppSettings { &mut self.app_settings }
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
            framework:    Framework::new(FocusedPane::App(TestPaneId::Main)),
            app_settings: (),
        };
        let _ = app.framework.toasts.push("hello", "body");

        assert!(app.framework.toasts.has_active());
    }
}

#[derive(Clone, Debug)]
pub struct TrackedItemView {
    pub label:           String,
    pub linger_progress: Option<f64>,
    pub elapsed:         Option<Duration>,
}

impl TrackedItemView {
    pub fn label(&self) -> &str { &self.label }
}

#[derive(Clone, Debug)]
pub struct ToastView {
    id:              ToastId,
    title:           String,
    body:            String,
    style:           ToastStyle,
    has_action:      bool,
    linger_progress: Option<f32>,
    remaining_secs:  Option<u64>,
    tracked_items:   Vec<TrackedItemView>,
    min_height:      u16,
    desired_height:  u16,
}

impl ToastView {
    pub fn id(&self) -> ToastId { self.id }

    pub fn title(&self) -> &str { &self.title }

    pub fn body(&self) -> &str { &self.body }

    pub fn style(&self) -> ToastStyle { self.style }

    pub fn has_action(&self) -> bool { self.has_action }

    pub fn linger_progress(&self) -> Option<f32> { self.linger_progress }

    pub fn remaining_secs(&self) -> Option<u64> { self.remaining_secs }

    pub fn tracked_items(&self) -> &[TrackedItemView] { &self.tracked_items }

    pub fn min_height(&self) -> u16 { self.min_height }

    pub fn desired_height(&self) -> u16 { self.desired_height }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToastHitbox {
    pub id:         ToastId,
    pub card_rect:  Rect,
    pub close_rect: Rect,
}

pub struct Toasts<Ctx: AppContext> {
    next_id:      u64,
    entries:      Vec<Toast<Ctx>>,
    pub viewport: Viewport,
    hits:         Vec<ToastHitbox>,
    _ctx:         PhantomData<fn(&Ctx)>,
}

impl<Ctx: AppContext> Default for Toasts<Ctx> {
    fn default() -> Self { Self::new() }
}

impl<Ctx: AppContext> Toasts<Ctx> {
    pub fn new() -> Self {
        Self {
            next_id:  1,
            entries:  Vec::new(),
            viewport: Viewport::default(),
            hits:     Vec::new(),
            _ctx:     PhantomData,
        }
    }

    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId {
        self.push_persistent_styled(title, body, ToastStyle::Normal, None, 1)
    }

    pub fn push_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
    ) -> ToastId {
        self.push_persistent_styled(title, body, style, None, 1)
    }

    pub fn push_with_action(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        action: Ctx::ToastAction,
    ) -> ToastId {
        self.push_persistent_styled(title, body, ToastStyle::Normal, Some(action), 1)
    }

    pub fn push_timed(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_timed_styled(title, body, timeout, min_interior_lines, ToastStyle::Normal)
    }

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
            title.into(),
            ToastBody::from(body.into()),
            style,
            ToastLifetime::Timed {
                timeout_at: now + timeout,
            },
            None,
            min_interior_lines,
            Duration::ZERO,
            now,
        )
    }

    pub fn push_task(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        min_interior_lines: usize,
    ) -> ToastTaskId {
        let id = ToastTaskId(self.next_id);
        let now = Instant::now();
        self.push_entry(
            title.into(),
            ToastBody::from(body.into()),
            ToastStyle::Normal,
            ToastLifetime::Task {
                task_id: id,
                status:  ToastTaskStatus::Running,
            },
            None,
            min_interior_lines,
            Duration::ZERO,
            now,
        );
        id
    }

    pub fn start_task(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastTaskId {
        self.push_task(title, body, 1)
    }

    pub fn push_persistent(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action: Option<Ctx::ToastAction>,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_entry(
            title.into(),
            ToastBody::from(body.into()),
            style,
            ToastLifetime::Persistent,
            action,
            min_interior_lines,
            Duration::ZERO,
            Instant::now(),
        )
    }

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

    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let now = Instant::now();
        let Some(toast) = self.entries.iter_mut().find(|toast| toast.id == id) else {
            return false;
        };
        toast.phase = ToastPhase::Exiting { started_at: now };
        true
    }

    pub fn dismiss_focused(&mut self) -> bool {
        self.focused_toast_id().is_some_and(|id| self.dismiss(id))
    }

    pub fn focused_id(&self) -> Option<ToastId> { self.focused_toast_id() }

    pub fn focused_toast_id(&self) -> Option<ToastId> {
        self.active_now()
            .get(self.viewport.pos())
            .map(ToastView::id)
    }

    pub fn has_active(&self) -> bool {
        let now = Instant::now();
        self.entries.iter().any(|toast| toast.is_live(now))
    }

    pub fn active(&self) -> &[Toast<Ctx>] { &self.entries }

    pub fn active_now(&self) -> Vec<ToastView> {
        self.active_views(Instant::now(), &ToastSettings::default())
    }

    pub fn active_views(&self, now: Instant, settings: &ToastSettings) -> Vec<ToastView> {
        self.entries
            .iter()
            .filter(|toast| toast.is_renderable(now, settings))
            .map(|toast| toast.view(now, settings))
            .collect()
    }

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

    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        toast.body = ToastBody::from(body.into());
        true
    }

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

    pub fn is_alive(&self, id: ToastId) -> bool {
        let now = Instant::now();
        self.entries
            .iter()
            .find(|toast| toast.id == id)
            .is_some_and(|toast| toast.is_live(now))
    }

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

    pub fn tracked_item_count(&self, task_id: ToastTaskId) -> usize {
        self.toast_for_task(task_id)
            .map(|toast| toast.tracked_items.len())
            .unwrap_or_default()
    }

    pub fn complete_missing_items(
        &mut self,
        task_id: ToastTaskId,
        active_keys: &HashSet<String>,
    ) -> bool {
        let now = Instant::now();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let mut changed = false;
        for item in &mut toast.tracked_items {
            if item.completed_at.is_none() && !active_keys.contains(item.key.as_str()) {
                item.completed_at = Some(now);
                changed = true;
            }
        }
        changed
    }

    pub fn add_new_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        items: &[TrackedItem],
        item_linger: Duration,
    ) -> bool {
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let existing_keys: HashSet<String> = toast
            .tracked_items
            .iter()
            .map(|item| item.key.as_str().to_owned())
            .collect();
        let mut changed = false;
        for item in items {
            if !existing_keys.contains(item.key.as_str()) {
                toast.tracked_items.push(item.clone());
                changed = true;
            }
        }
        toast.item_linger = item_linger;
        changed
    }

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

    pub fn mark_item_completed(&mut self, task_id: ToastTaskId, key: &str) -> bool {
        let now = Instant::now();
        let Some(toast) = self.toast_for_task_mut(task_id) else {
            return false;
        };
        let Some(item) = toast
            .tracked_items
            .iter_mut()
            .find(|item| item.key.as_str() == key)
        else {
            return false;
        };
        item.completed_at = Some(now);
        true
    }

    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) -> bool {
        self.mark_item_completed(task_id, key)
    }

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

    pub fn reset_to_first(&mut self) {
        if self.has_active() {
            self.viewport.set_pos(0);
        }
    }

    pub fn reset_to_last(&mut self) {
        let len = self.active_now().len();
        if len > 0 {
            self.viewport.set_pos(len - 1);
        }
    }

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

    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome { self.handle_key_command(bind).0 }

    pub const fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    pub fn defaults() -> Bindings<ToastsAction> {
        crate::bindings! {
            KeyCode::Enter => ToastsAction::Activate,
        }
    }

    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|action| (BarRegion::PaneAction, BarSlot::Single(action)))
            .collect()
    }

    pub fn hits(&self) -> &[ToastHitbox] { &self.hits }

    pub fn set_hits(&mut self, hits: Vec<ToastHitbox>) { self.hits = hits; }

    fn push_entry(
        &mut self,
        title: String,
        body: ToastBody,
        style: ToastStyle,
        lifetime: ToastLifetime,
        action: Option<Ctx::ToastAction>,
        min_interior_lines: usize,
        item_linger: Duration,
        now: Instant,
    ) -> ToastId {
        let id = ToastId(self.next_id);
        self.next_id += 1;
        self.entries.push(Toast {
            id,
            title,
            body,
            style,
            lifetime,
            phase: ToastPhase::Visible,
            action,
            tracked_items: Vec::new(),
            created_at: now,
            min_interior_lines,
            item_linger,
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

impl<Ctx: AppContext> Toast<Ctx> {
    fn task_id(&self) -> Option<ToastTaskId> {
        match self.lifetime {
            ToastLifetime::Task { task_id, .. } => Some(task_id),
            ToastLifetime::Timed { .. } | ToastLifetime::Persistent => None,
        }
    }

    fn is_live(&self, now: Instant) -> bool {
        matches!(self.phase, ToastPhase::Visible) && !self.should_exit(now)
    }

    fn is_renderable(&self, now: Instant, settings: &ToastSettings) -> bool {
        match self.phase {
            ToastPhase::Visible => !self.should_exit(now),
            ToastPhase::Exiting { started_at } => self.exit_lines(now, settings, started_at) > 0,
        }
    }

    fn should_exit(&self, now: Instant) -> bool {
        match self.lifetime {
            ToastLifetime::Timed { timeout_at } => now >= timeout_at,
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => now >= finished_at + linger,
            ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => false,
        }
    }

    fn view(&self, now: Instant, settings: &ToastSettings) -> ToastView {
        let min_height = self.min_height();
        let desired_height = self.current_visible_lines(now, settings).max(min_height);
        ToastView {
            id: self.id,
            title: self.title.clone(),
            body: self.body.as_text(),
            style: self.style,
            has_action: self.action.is_some(),
            linger_progress: self.linger_progress(now),
            remaining_secs: self.remaining_secs(now),
            tracked_items: self
                .tracked_items
                .iter()
                .map(|item| {
                    let elapsed = item.started_at.map(|started_at| {
                        let ended_at = item.completed_at.unwrap_or(now);
                        ended_at.saturating_duration_since(started_at)
                    });
                    let linger_progress = item.completed_at.and_then(|completed_at| {
                        (!self.item_linger.is_zero()).then(|| {
                            now.saturating_duration_since(completed_at).as_secs_f64()
                                / self.item_linger.as_secs_f64()
                        })
                    });
                    TrackedItemView {
                        label: item.label.clone(),
                        linger_progress,
                        elapsed,
                    }
                })
                .collect(),
            min_height,
            desired_height,
        }
    }

    fn min_height(&self) -> u16 { (self.min_interior_lines + 2).try_into().unwrap_or(u16::MAX) }

    fn current_visible_lines(&self, now: Instant, settings: &ToastSettings) -> u16 {
        let target = self.target_height(settings);
        match self.phase {
            ToastPhase::Visible => {
                let elapsed = now.saturating_duration_since(self.created_at);
                let line_ms = settings
                    .animation
                    .entrance_duration
                    .get()
                    .as_millis()
                    .max(1);
                let lines = (elapsed.as_millis() / line_ms) + 1;
                (lines as u16).min(target).max(self.min_height())
            },
            ToastPhase::Exiting { started_at } => self.exit_lines(now, settings, started_at),
        }
    }

    fn exit_lines(&self, now: Instant, settings: &ToastSettings, started_at: Instant) -> u16 {
        let target = self.target_height(settings);
        let elapsed = now.saturating_duration_since(started_at);
        let line_ms = settings.animation.exit_duration.get().as_millis().max(1);
        let hidden = (elapsed.as_millis() / line_ms) as u16;
        target.saturating_sub(hidden)
    }

    fn target_height(&self, settings: &ToastSettings) -> u16 {
        let width = toast_body_width(settings);
        let body_lines = self.body.wrapped_line_count(width);
        let item_lines = if self.tracked_items.is_empty() {
            body_lines
        } else {
            self.tracked_items.len()
        };
        let elapsed_line = usize::from(matches!(self.lifetime, ToastLifetime::Task { .. }));
        let interior = self.min_interior_lines.max(item_lines + elapsed_line);
        (interior + 2).try_into().unwrap_or(u16::MAX)
    }

    fn linger_progress(&self, now: Instant) -> Option<f32> {
        let ToastLifetime::Task {
            status:
                ToastTaskStatus::Finished {
                    finished_at,
                    linger,
                },
            ..
        } = self.lifetime
        else {
            return None;
        };
        if linger.is_zero() {
            return Some(1.0);
        }
        let elapsed = now.saturating_duration_since(finished_at);
        Some((elapsed.as_secs_f32() / linger.as_secs_f32()).clamp(0.0, 1.0))
    }

    fn remaining_secs(&self, now: Instant) -> Option<u64> {
        match self.lifetime {
            ToastLifetime::Timed { timeout_at } => timeout_at
                .checked_duration_since(now)
                .map(|duration| duration.as_secs().saturating_add(1)),
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => (finished_at + linger)
                .checked_duration_since(now)
                .map(|duration| duration.as_secs().saturating_add(1)),
            ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => None,
        }
    }
}

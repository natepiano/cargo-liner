use std::time::Duration;
use std::time::Instant;

use super::Toast;
use super::ToastBody;
use super::ToastDismissal;
use super::ToastId;
use super::ToastLifetime;
use super::ToastPhase;
use super::ToastSpec;
use super::ToastStyle;
use super::ToastTaskId;
use super::ToastTaskStatus;
use super::Toasts;
use crate::AppContext;

impl<Ctx: AppContext> Toasts<Ctx> {
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

    /// Push a status toast that auto-closes after
    /// [`ToastSettings::status_toast_visible`](crate::ToastSettings::status_toast_visible).
    pub fn push_status(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId {
        self.push_status_styled(title, body, ToastStyle::Normal)
    }

    /// Push a status toast with an explicit style.
    pub fn push_status_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
    ) -> ToastId {
        let timeout = self.settings().status_toast_visible.get();
        self.push_timed_styled(title, body, timeout, 1, style)
    }

    /// Push a timed informational toast with an explicit timeout.
    pub fn push_timed(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
        min_interior_lines: usize,
    ) -> ToastId {
        self.push_timed_styled(title, body, timeout, min_interior_lines, ToastStyle::Normal)
    }

    /// Push a timed toast with an explicit style and timeout.
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
                title: title.into(),
                body: ToastBody::from(body.into()),
                style,
                lifetime: ToastLifetime::Timed {
                    timeout_at: now + timeout,
                },
                action: None,
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
                title: title.into(),
                body: ToastBody::from(body.into()),
                style: ToastStyle::Normal,
                lifetime: ToastLifetime::Task {
                    task_id: id,
                    status:  ToastTaskStatus::Running,
                },
                action: None,
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
                title: title.into(),
                body: ToastBody::from(body.into()),
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
    ///
    /// If the toast is already in [`ToastPhase::Exiting`] the
    /// existing `started_at` is preserved — re-dismissing a
    /// fading toast must not restart its exit animation from the
    /// beginning, which would visibly "pop" the toast back to
    /// full size. Records the dismissal as
    /// [`ToastDismissal::ClosedByUser`] either way, so
    /// [`Self::reactivate_task`](super::Toasts::reactivate_task) does
    /// not bring the toast back when its tracker keeps reporting
    /// in-flight work.
    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let now = Instant::now();
        let Some(toast) = self.entries.iter_mut().find(|toast| toast.id == id) else {
            return false;
        };
        if matches!(toast.phase, ToastPhase::Visible) {
            toast.phase = ToastPhase::Exiting { started_at: now };
        }
        toast.dismissal = ToastDismissal::ClosedByUser;
        true
    }

    /// Start dismissing the currently focused toast.
    pub fn dismiss_focused(&mut self) -> bool {
        self.focused_toast_id().is_some_and(|id| self.dismiss(id))
    }

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
            dismissal: ToastDismissal::Open,
            action: spec.action,
            tracked_items: Vec::new(),
            created_at: now,
            min_interior_lines: spec.min_interior_lines,
            item_linger: spec.item_linger,
        });
        self.sync_viewport_len();
        id
    }
}

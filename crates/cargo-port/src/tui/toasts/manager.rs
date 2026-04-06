use std::time::Duration;
use std::time::Instant;

use crate::tui::constants::TOAST_HEIGHT;
use crate::tui::constants::TOAST_LINE_REVEAL_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastTaskId(pub u64);

#[derive(Clone, Debug)]
struct Toast {
    id:              u64,
    title:           String,
    body:            String,
    timeout_at:      Option<Instant>,
    task_id:         Option<ToastTaskId>,
    dismissed:       bool,
    finished_task:   bool,
    created_at:      Instant,
    exit_started_at: Option<Instant>,
}

impl Toast {
    /// A toast is alive while it is entering, fully visible, or animating
    /// out.  It becomes dead only once the exit animation finishes.
    fn is_alive(&self, now: Instant) -> bool {
        self.exit_started_at.map_or_else(
            || {
                !self.dismissed
                    && !self.finished_task
                    && self.timeout_at.is_none_or(|deadline| deadline > now)
            },
            |exit_start| exit_lines(now, exit_start) > 0,
        )
    }

    /// Should this toast begin its exit animation right now?
    fn should_exit(&self, now: Instant) -> bool {
        self.exit_started_at.is_none()
            && (self.dismissed
                || self.finished_task
                || self.timeout_at.is_some_and(|deadline| now >= deadline))
    }

    fn current_visible_lines(&self, now: Instant) -> u16 {
        if let Some(exit_start) = self.exit_started_at {
            return exit_lines(now, exit_start);
        }
        let elapsed_lines = elapsed_line_count(now.duration_since(self.created_at));
        // Start at 2 (top+bottom border) — height=1 renders a stray
        // corner glyph that clashes with the window border.
        (2 + elapsed_lines).min(TOAST_HEIGHT)
    }
}

fn exit_lines(now: Instant, exit_start: Instant) -> u16 {
    if now >= exit_start {
        let remaining =
            TOAST_HEIGHT.saturating_sub(elapsed_line_count(now.duration_since(exit_start)));
        // Skip height=1: a single-line Block renders a stray corner glyph
        // that clashes with the window border. Jump from 2 → 0.
        if remaining == 1 { 0 } else { remaining }
    } else {
        TOAST_HEIGHT
    }
}

/// How many full `TOAST_LINE_REVEAL_MS` intervals fit in `elapsed`.
///
/// Uses `as_secs()` + `subsec_millis()` to stay within u64 arithmetic
/// and avoid truncation casts from `as_millis() -> u128`.
fn elapsed_line_count(elapsed: Duration) -> u16 {
    let ms = elapsed
        .as_secs()
        .saturating_mul(1000)
        .saturating_add(u64::from(elapsed.subsec_millis()));
    u16::try_from(ms / TOAST_LINE_REVEAL_MS).unwrap_or(u16::MAX)
}

#[derive(Clone, Copy, Debug)]
pub struct ToastView<'a> {
    id:            u64,
    title:         &'a str,
    body:          &'a str,
    visible_lines: u16,
}

impl<'a> ToastView<'a> {
    pub const fn id(&self) -> u64 { self.id }

    pub const fn title(&self) -> &'a str { self.title }

    pub const fn body(&self) -> &'a str { self.body }

    pub const fn visible_lines(&self) -> u16 { self.visible_lines }
}

#[derive(Default)]
pub struct ToastManager {
    next_id: u64,
    toasts:  Vec<Toast>,
}

impl ToastManager {
    const fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn push_timed(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
    ) -> u64 {
        let id = self.alloc_id();
        let now = Instant::now();
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body: body.into(),
            timeout_at: Some(now + timeout),
            task_id: None,
            dismissed: false,
            finished_task: false,
            created_at: now,
            exit_started_at: None,
        });
        id
    }

    pub fn push_task(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastTaskId {
        let id = self.alloc_id();
        let task_id = ToastTaskId(id);
        let now = Instant::now();
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body: body.into(),
            timeout_at: None,
            task_id: Some(task_id),
            dismissed: false,
            finished_task: false,
            created_at: now,
            exit_started_at: None,
        });
        task_id
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(toast) = self.toasts.iter_mut().find(|toast| toast.id == id) {
            toast.dismissed = true;
        }
    }

    pub fn finish_task(&mut self, task_id: ToastTaskId) {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                toast.finished_task = true;
            }
        }
    }

    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) {
        let body = body.into();
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                toast.body.clone_from(&body);
            }
        }
    }

    pub fn prune(&mut self, now: Instant) {
        // Start exit animations for toasts that should begin exiting.
        for toast in &mut self.toasts {
            if toast.should_exit(now) {
                toast.exit_started_at = Some(now);
            }
        }
        // Remove toasts whose exit animation has completed.
        self.toasts.retain(|toast| toast.is_alive(now));
    }

    pub fn active(&self, now: Instant) -> Vec<ToastView<'_>> {
        self.toasts
            .iter()
            .filter(|toast| toast.is_alive(now))
            .map(|toast| ToastView {
                id:            toast.id,
                title:         &toast.title,
                body:          &toast.body,
                visible_lines: toast.current_visible_lines(now),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Duration long enough for the full exit animation to complete.
    const EXIT_ANIMATION: Duration =
        Duration::from_millis(TOAST_HEIGHT as u64 * TOAST_LINE_REVEAL_MS + 1);

    #[test]
    fn timed_toast_expires() {
        let mut manager = ToastManager::default();
        manager.push_timed("settings", "updated", Duration::from_millis(10));
        assert_eq!(manager.active(Instant::now()).len(), 1);

        // Prune after timeout — starts exit animation but toast is still alive.
        let after_timeout = Instant::now() + Duration::from_millis(20);
        manager.prune(after_timeout);
        assert_eq!(manager.active(after_timeout).len(), 1);

        // After the exit animation completes, the toast is fully removed.
        let after_exit = after_timeout + EXIT_ANIMATION;
        manager.prune(after_exit);
        assert!(manager.active(after_exit).is_empty());
    }

    #[test]
    fn task_toast_finishes_independently() {
        let mut manager = ToastManager::default();
        let task = manager.push_task("cargo clean", "~/rust/bevy");
        assert_eq!(manager.active(Instant::now()).len(), 1);

        manager.finish_task(task);
        let now = Instant::now();
        manager.prune(now);
        // Still alive during exit animation.
        assert_eq!(manager.active(now).len(), 1);

        let after_exit = now + EXIT_ANIMATION;
        manager.prune(after_exit);
        assert!(manager.active(after_exit).is_empty());
    }

    #[test]
    fn task_toast_body_can_be_updated() {
        let mut manager = ToastManager::default();
        let task = manager.push_task("startup git", "loading");
        manager.update_task_body(task, "2 remaining");
        let active = manager.active(Instant::now());
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].body(), "2 remaining");
    }
}

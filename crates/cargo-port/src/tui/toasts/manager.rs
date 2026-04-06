use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use crate::tui::constants::TOAST_LINE_REVEAL_MS;
use crate::tui::constants::TOAST_MAX_HEIGHT;
use crate::tui::constants::TOAST_MIN_HEIGHT;
use crate::tui::constants::TOAST_WIDTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastTaskId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ToastPersistence {
    #[default]
    Timed,
    Task,
    Permanent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastStyle {
    #[default]
    Normal,
    Error,
}

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
    persistence:     ToastPersistence,
    style:           ToastStyle,
    action_path:     Option<PathBuf>,
    target_height:   u16,
}

impl Toast {
    /// A toast is alive while it is entering, fully visible, or animating
    /// out.  It becomes dead only once the exit animation finishes.
    fn is_alive(&self, now: Instant) -> bool {
        self.exit_started_at.map_or_else(
            || {
                if self.dismissed {
                    return false;
                }
                match self.persistence {
                    ToastPersistence::Permanent => true,
                    ToastPersistence::Task => !self.finished_task,
                    ToastPersistence::Timed => {
                        self.timeout_at.is_none_or(|deadline| deadline > now)
                    },
                }
            },
            |exit_start| exit_lines(now, exit_start, self.target_height) > 0,
        )
    }

    /// Should this toast begin its exit animation right now?
    fn should_exit(&self, now: Instant) -> bool {
        if self.exit_started_at.is_some() {
            return false;
        }
        if self.dismissed {
            return true;
        }
        match self.persistence {
            ToastPersistence::Permanent => false,
            ToastPersistence::Task => self.finished_task,
            ToastPersistence::Timed => self.timeout_at.is_some_and(|deadline| now >= deadline),
        }
    }

    fn current_visible_lines(&self, now: Instant) -> u16 {
        if let Some(exit_start) = self.exit_started_at {
            return exit_lines(now, exit_start, self.target_height);
        }
        let elapsed_lines = elapsed_line_count(now.duration_since(self.created_at));
        // Start at 2 (top+bottom border) — height=1 renders a stray
        // corner glyph that clashes with the window border.
        (2 + elapsed_lines).min(self.target_height)
    }
}

fn exit_lines(now: Instant, exit_start: Instant, target_height: u16) -> u16 {
    if now >= exit_start {
        let remaining =
            target_height.saturating_sub(elapsed_line_count(now.duration_since(exit_start)));
        // Skip height=1: a single-line Block renders a stray corner glyph
        // that clashes with the window border. Jump from 2 → 0.
        if remaining == 1 { 0 } else { remaining }
    } else {
        target_height
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

#[derive(Clone, Debug)]
pub struct ToastView<'a> {
    id:            u64,
    title:         &'a str,
    body:          &'a str,
    visible_lines: u16,
    style:         ToastStyle,
    action_path:   Option<&'a Path>,
}

impl<'a> ToastView<'a> {
    pub const fn id(&self) -> u64 { self.id }

    pub const fn title(&self) -> &'a str { self.title }

    pub const fn body(&self) -> &'a str { self.body }

    pub const fn visible_lines(&self) -> u16 { self.visible_lines }

    pub const fn style(&self) -> ToastStyle { self.style }

    pub const fn action_path(&self) -> Option<&Path> { self.action_path }
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
        let body = body.into();
        let target_height = compute_target_height(&body);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: Some(now + timeout),
            task_id: None,
            dismissed: false,
            finished_task: false,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Timed,
            style: ToastStyle::Normal,
            action_path: None,
            target_height,
        });
        id
    }

    pub fn push_task(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastTaskId {
        let id = self.alloc_id();
        let task_id = ToastTaskId(id);
        let now = Instant::now();
        let body = body.into();
        let target_height = compute_target_height(&body);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: None,
            task_id: Some(task_id),
            dismissed: false,
            finished_task: false,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Task,
            style: ToastStyle::Normal,
            action_path: None,
            target_height,
        });
        task_id
    }

    pub fn push_persistent(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action_path: Option<PathBuf>,
    ) -> u64 {
        let id = self.alloc_id();
        let now = Instant::now();
        let body = body.into();
        let target_height = compute_target_height(&body);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: None,
            task_id: None,
            dismissed: false,
            finished_task: false,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Permanent,
            style,
            action_path,
            target_height,
        });
        id
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
        let target_height = compute_target_height(&body);
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                toast.body.clone_from(&body);
                toast.target_height = target_height;
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
                style:         toast.style,
                action_path:   toast.action_path.as_deref(),
            })
            .collect()
    }
}

/// Compute the target (fully-revealed) height for a toast based on its
/// body content.  Height = 2 (borders, title is on the top border) +
/// body lines, clamped to `[TOAST_MIN_HEIGHT, TOAST_MAX_HEIGHT]`.
fn compute_target_height(body: &str) -> u16 {
    // Inner width is toast width minus 2 for borders.
    let inner_width = usize::from(TOAST_WIDTH.saturating_sub(2));
    let body_lines = if body.is_empty() {
        1
    } else {
        body.lines()
            .map(|line| {
                let width = unicode_width::UnicodeWidthStr::width(line);
                if width == 0 {
                    1
                } else {
                    width.div_ceil(inner_width.max(1))
                }
            })
            .sum::<usize>()
    };
    let raw = u16::try_from(2 + body_lines).unwrap_or(u16::MAX);
    raw.clamp(TOAST_MIN_HEIGHT, TOAST_MAX_HEIGHT)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Duration long enough for the full exit animation to complete.
    const EXIT_ANIMATION: Duration =
        Duration::from_millis(TOAST_MAX_HEIGHT as u64 * TOAST_LINE_REVEAL_MS + 1);

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

    #[test]
    fn permanent_toast_stays_after_prune() {
        let mut manager = ToastManager::default();
        manager.push_persistent("error", "bad keymap", ToastStyle::Error, None);

        // Prune many times — permanent toast stays.
        let later = Instant::now() + Duration::from_secs(3600);
        manager.prune(later);
        assert_eq!(manager.active(later).len(), 1);
    }

    #[test]
    fn permanent_toast_dismissed_by_user() {
        let mut manager = ToastManager::default();
        let id = manager.push_persistent("error", "bad keymap", ToastStyle::Error, None);
        manager.dismiss(id);

        let now = Instant::now();
        manager.prune(now);
        // Still alive during exit animation.
        assert_eq!(manager.active(now).len(), 1);

        let after_exit = now + EXIT_ANIMATION;
        manager.prune(after_exit);
        assert!(manager.active(after_exit).is_empty());
    }

    #[test]
    fn toast_view_exposes_style() {
        let mut manager = ToastManager::default();
        manager.push_persistent("error", "bad", ToastStyle::Error, None);
        let active = manager.active(Instant::now());
        assert_eq!(active[0].style(), ToastStyle::Error);
    }

    #[test]
    fn toast_view_exposes_action_path() {
        let mut manager = ToastManager::default();
        let path = PathBuf::from("/tmp/keymap.toml");
        manager.push_persistent("error", "bad", ToastStyle::Error, Some(path.clone()));
        let active = manager.active(Instant::now());
        assert_eq!(active[0].action_path(), Some(path.as_path()));
    }

    #[test]
    fn variable_height_short_body() {
        // 2 borders + 1 title + 1 body line = 4, clamped to min 5
        assert_eq!(compute_target_height("short"), TOAST_MIN_HEIGHT);
    }

    #[test]
    fn variable_height_long_body() {
        let body = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        // 2 + 1 + 10 = 13, clamped to max 12
        assert_eq!(compute_target_height(body), TOAST_MAX_HEIGHT);
    }

    #[test]
    fn variable_height_multiline_body() {
        let body = "line1\nline2\nline3";
        // 2 (borders, title on top border) + 3 body lines = 5
        assert_eq!(compute_target_height(body), 5);
    }
}

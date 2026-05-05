use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::constants::TOAST_LINE_REVEAL_MS;
use crate::tui::constants::TOAST_WIDTH;
use crate::tui::interaction::ToastHitbox;
use crate::tui::pane::Viewport;

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
    Warning,
    Error,
}

#[derive(Clone, Debug)]
struct Toast {
    id:                 u64,
    title:              String,
    body:               String,
    timeout_at:         Option<Instant>,
    task_id:            Option<ToastTaskId>,
    dismissed:          bool,
    finished_task:      bool,
    finished_at:        Option<Instant>,
    linger_duration:    Option<Duration>,
    created_at:         Instant,
    exit_started_at:    Option<Instant>,
    persistence:        ToastPersistence,
    style:              ToastStyle,
    action_path:        Option<AbsolutePath>,
    target_height:      u16,
    min_interior_lines: u16,
    /// Per-item linger duration (used while toast is still active).
    item_linger:        Option<Duration>,
    /// Tracked items for task toasts — each item can linger after completion.
    tracked_items:      Vec<TrackedItem>,
}

#[derive(Clone, Debug)]
pub struct TrackedItemKey(String);

impl TrackedItemKey {
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for TrackedItemKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
}

impl From<AbsolutePath> for TrackedItemKey {
    fn from(value: AbsolutePath) -> Self { Self(value.to_string()) }
}

impl From<&AbsolutePath> for TrackedItemKey {
    fn from(value: &AbsolutePath) -> Self { Self(value.to_string()) }
}

impl From<OwnerRepo> for TrackedItemKey {
    fn from(value: OwnerRepo) -> Self { Self(value.to_string()) }
}

impl From<&OwnerRepo> for TrackedItemKey {
    fn from(value: &OwnerRepo) -> Self { Self(value.to_string()) }
}

impl From<&str> for TrackedItemKey {
    fn from(value: &str) -> Self { Self(String::from(value)) }
}

#[derive(Clone, Debug)]
pub struct TrackedItem {
    pub label:        String,
    pub key:          TrackedItemKey,
    pub started_at:   Option<Instant>,
    pub completed_at: Option<Instant>,
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
    id:                 u64,
    title:              &'a str,
    body:               &'a str,
    visible_lines:      u16,
    style:              ToastStyle,
    action_path:        Option<&'a Path>,
    min_interior_lines: u16,
    target_height:      u16,
    /// 0.0 = just finished, 1.0 = linger complete, about to exit. None = not lingering.
    linger_progress:    Option<f64>,
    tracked_items:      Vec<TrackedItemView>,
    /// Seconds remaining before the toast begins its exit animation. `None`
    /// for permanent/task toasts that have no timeout.
    remaining_secs:     Option<u64>,
}

/// A tracked item in a toast, with per-item linger progress.
#[derive(Clone, Debug)]
pub struct TrackedItemView {
    pub label:           String,
    /// None = pending. Some(0.0..1.0) = lingering with strikethrough progress.
    pub linger_progress: Option<f64>,
    /// Elapsed duration since the item started. `None` if no start time recorded.
    pub elapsed:         Option<Duration>,
}

impl<'a> ToastView<'a> {
    pub const fn id(&self) -> u64 { self.id }

    pub const fn title(&self) -> &'a str { self.title }

    pub const fn body(&self) -> &'a str { self.body }

    pub const fn visible_lines(&self) -> u16 { self.visible_lines }

    pub const fn style(&self) -> ToastStyle { self.style }

    pub const fn action_path(&self) -> Option<&Path> { self.action_path }

    /// Linger progress: 0.0 = just finished, 1.0 = about to exit. None if not lingering.
    pub const fn linger_progress(&self) -> Option<f64> { self.linger_progress }

    /// Seconds remaining before exit. `None` for non-timed toasts.
    pub const fn remaining_secs(&self) -> Option<u64> { self.remaining_secs }

    /// Tracked items with per-item linger progress.
    pub fn tracked_items(&self) -> &[TrackedItemView] { &self.tracked_items }

    /// Minimum height: 2 (borders) + `min_interior_lines`.
    pub const fn min_height(&self) -> u16 { 2 + self.min_interior_lines }

    /// Full desired height based on body content.
    pub const fn desired_height(&self) -> u16 { self.target_height }
}

#[derive(Default)]
pub struct ToastManager {
    next_id:  u64,
    toasts:   Vec<Toast>,
    /// Per-pane cursor for the toasts overlay. Phase 14 absorption:
    /// the toasts viewport lives with its data, not on a separate
    /// `ToastsPane` wrapper.
    viewport: Viewport,
    /// Per-toast hit rects recorded each frame by `render_toasts`
    /// (card body + close `[x]` action). Walked top-down by
    /// `Hittable::hit_test_at`; the action region wins over the body.
    hits:     Vec<ToastHitbox>,
}

impl ToastManager {
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub fn set_hits(&mut self, hits: Vec<ToastHitbox>) { self.hits = hits; }

    #[cfg(test)]
    pub fn hits(&self) -> &[ToastHitbox] { &self.hits }

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
        min_interior_lines: u16,
    ) -> u64 {
        self.push_timed_styled(title, body, timeout, min_interior_lines, ToastStyle::Normal)
    }

    pub fn push_timed_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
        min_interior_lines: u16,
        style: ToastStyle,
    ) -> u64 {
        let id = self.alloc_id();
        let now = Instant::now();
        let body = body.into();
        let target_height = compute_target_height(&body, min_interior_lines);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: Some(now + timeout),
            task_id: None,
            dismissed: false,
            finished_task: false,
            finished_at: None,
            linger_duration: None,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Timed,
            style,
            action_path: None,
            target_height,
            min_interior_lines,
            item_linger: None,
            tracked_items: Vec::new(),
        });
        id
    }

    pub fn push_task(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        min_interior_lines: u16,
    ) -> ToastTaskId {
        let id = self.alloc_id();
        let task_id = ToastTaskId(id);
        let now = Instant::now();
        let body = body.into();
        let target_height = compute_target_height(&body, min_interior_lines);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: None,
            task_id: Some(task_id),
            dismissed: false,
            finished_task: false,
            finished_at: None,
            linger_duration: None,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Task,
            style: ToastStyle::Normal,
            action_path: None,
            target_height,
            min_interior_lines,
            item_linger: None,
            tracked_items: Vec::new(),
        });
        task_id
    }

    pub fn push_persistent(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action_path: Option<AbsolutePath>,
        min_interior_lines: u16,
    ) -> u64 {
        let id = self.alloc_id();
        let now = Instant::now();
        let body = body.into();
        let target_height = compute_target_height(&body, min_interior_lines);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body,
            timeout_at: None,
            task_id: None,
            dismissed: false,
            finished_task: false,
            finished_at: None,
            linger_duration: None,
            created_at: now,
            exit_started_at: None,
            persistence: ToastPersistence::Permanent,
            style,
            action_path,
            target_height,
            min_interior_lines,
            item_linger: None,
            tracked_items: Vec::new(),
        });
        id
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(toast) = self.toasts.iter_mut().find(|toast| toast.id == id) {
            toast.dismissed = true;
        }
    }

    pub fn finish_task(&mut self, task_id: ToastTaskId, linger: Duration) {
        let now = Instant::now();
        let deadline = now + linger;
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) && !toast.finished_task {
                toast.finished_task = true;
                toast.finished_at = Some(now);
                toast.linger_duration = Some(linger);
                toast.timeout_at = Some(deadline);
                toast.persistence = ToastPersistence::Timed;
            }
        }
    }

    /// Reactivate a finished or active task toast. If already active this is a
    /// no-op. Returns `true` if the toast was found (whether it needed
    /// reactivation or not).
    pub fn reactivate_task(&mut self, task_id: ToastTaskId) -> bool {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                if toast.finished_task {
                    toast.finished_task = false;
                    toast.finished_at = None;
                    toast.linger_duration = None;
                    toast.timeout_at = None;
                    toast.exit_started_at = None;
                    toast.persistence = ToastPersistence::Task;
                }
                return true;
            }
        }
        false
    }

    #[cfg(test)]
    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) {
        let body = body.into();
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                let target_height = compute_target_height(&body, toast.min_interior_lines);
                toast.body.clone_from(&body);
                toast.target_height = target_height;
            }
        }
    }

    /// Set tracked items for a task toast. Items are displayed instead of
    /// the plain body text. Completed items linger with strikethrough.
    pub fn set_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        items: &[TrackedItem],
        item_linger: Duration,
    ) {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                // Rebuild body from tracked items for height calc.
                let body: String = items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let target_height = compute_target_height(&body, toast.min_interior_lines);
                toast.body = body;
                toast.target_height = target_height;
                toast.item_linger = Some(item_linger);
                toast.tracked_items = items.to_vec();
            }
        }
    }

    /// Check if a toast with the given id is currently alive (present,
    /// not dismissed, not expired). Returns false if the id is unknown
    /// (e.g. the toast was manually dismissed and its exit animation
    /// completed, so it's been evicted from the Vec).
    pub fn is_alive(&self, id: u64) -> bool {
        let now = Instant::now();
        self.toasts
            .iter()
            .find(|toast| toast.id == id)
            .is_some_and(|toast| toast.is_alive(now))
    }

    /// Check if a task toast has been finished (countdown started).
    pub fn is_task_finished(&self, task_id: ToastTaskId) -> bool {
        self.toasts
            .iter()
            .find(|t| t.task_id == Some(task_id))
            .is_some_and(|t| t.finished_task)
    }

    /// Count remaining tracked items for a task toast.
    pub fn tracked_item_count(&self, task_id: ToastTaskId) -> usize {
        self.toasts
            .iter()
            .find(|t| t.task_id == Some(task_id))
            .map_or(0, |t| t.tracked_items.len())
    }

    /// Mark tracked items as completed if their key is NOT in the active set.
    pub fn complete_missing_items(&mut self, task_id: ToastTaskId, active_keys: &HashSet<String>) {
        let now = Instant::now();
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                for item in &mut toast.tracked_items {
                    if item.completed_at.is_none() && !active_keys.contains(item.key.as_str()) {
                        item.completed_at = Some(now);
                    }
                }
            }
        }
    }

    /// Add tracked items that aren't already tracked (matched by key).
    pub fn add_new_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        new_items: &[TrackedItem],
        item_linger: Duration,
    ) {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                let existing: HashSet<String> = toast
                    .tracked_items
                    .iter()
                    .map(|i| i.key.to_string())
                    .collect();
                for item in new_items {
                    if !existing.contains(item.key.as_str()) {
                        toast.tracked_items.push(item.clone());
                    }
                }
                toast.item_linger = Some(item_linger);
                let body: String = toast
                    .tracked_items
                    .iter()
                    .map(|i| i.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                toast.target_height = compute_target_height(&body, toast.min_interior_lines);
                toast.body = body;
                break;
            }
        }
    }

    /// Restart a tracked item — reset its `started_at` and clear
    /// `completed_at` so the spinner and duration counter restart.
    pub fn restart_tracked_item(
        &mut self,
        task_id: ToastTaskId,
        key: &TrackedItemKey,
        started_at: Instant,
    ) {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                for item in &mut toast.tracked_items {
                    if item.key.as_str() == key.as_str() {
                        item.started_at = Some(started_at);
                        item.completed_at = None;
                    }
                }
                break;
            }
        }
    }

    /// Mark a tracked item as completed by key.
    pub fn mark_item_completed(&mut self, task_id: ToastTaskId, key: &str) {
        let now = Instant::now();
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                for item in &mut toast.tracked_items {
                    if item.key.as_str() == key && item.completed_at.is_none() {
                        item.completed_at = Some(now);
                        break;
                    }
                }
            }
        }
    }

    /// Remove expired lingering items from tracked toasts.
    pub fn prune_tracked_items(&mut self, now: Instant, linger: Duration) {
        for toast in &mut self.toasts {
            if toast.tracked_items.is_empty() {
                continue;
            }
            let before = toast.tracked_items.len();
            toast.tracked_items.retain(|item| {
                item.completed_at
                    .is_none_or(|completed| now.duration_since(completed) < linger)
            });
            if toast.tracked_items.len() != before {
                let body: String = toast
                    .tracked_items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                toast.target_height = compute_target_height(&body, toast.min_interior_lines);
                toast.body = body;
                // All tracked items gone — auto-exit the toast immediately.
                if toast.tracked_items.is_empty() && toast.exit_started_at.is_none() {
                    toast.persistence = ToastPersistence::Timed;
                    toast.timeout_at = Some(now);
                }
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

    pub fn active_now(&self) -> Vec<ToastView<'_>> { self.active(Instant::now()) }

    pub fn active(&self, now: Instant) -> Vec<ToastView<'_>> {
        self.toasts
            .iter()
            .filter(|toast| toast.is_alive(now))
            .map(|toast| {
                let linger_progress =
                    toast
                        .finished_at
                        .zip(toast.linger_duration)
                        .map(|(finished_at, linger)| {
                            let elapsed = now.duration_since(finished_at).as_secs_f64();
                            let total = linger.as_secs_f64();
                            if total <= 0.0 {
                                1.0
                            } else {
                                (elapsed / total).clamp(0.0, 1.0)
                            }
                        });
                let tracked_items: Vec<TrackedItemView> = toast
                    .tracked_items
                    .iter()
                    .map(|item| {
                        let item_progress = item.completed_at.and_then(|completed| {
                            toast.item_linger.map(|linger| {
                                let elapsed = now.duration_since(completed).as_secs_f64();
                                let total = linger.as_secs_f64();
                                if total <= 0.0 {
                                    1.0
                                } else {
                                    (elapsed / total).clamp(0.0, 1.0)
                                }
                            })
                        });
                        let elapsed = item.started_at.map(|started| {
                            let end = item.completed_at.unwrap_or(now);
                            end.duration_since(started)
                        });
                        TrackedItemView {
                            label: item.label.clone(),
                            linger_progress: item_progress,
                            elapsed,
                        }
                    })
                    .collect();
                let remaining_secs = toast.timeout_at.and_then(|deadline| {
                    if deadline > now {
                        Some(deadline.duration_since(now).as_secs().saturating_add(1))
                    } else {
                        None
                    }
                });
                ToastView {
                    id: toast.id,
                    title: &toast.title,
                    body: &toast.body,
                    visible_lines: toast.current_visible_lines(now),
                    style: toast.style,
                    action_path: toast.action_path.as_deref(),
                    min_interior_lines: toast.min_interior_lines,
                    target_height: toast.target_height,
                    linger_progress,
                    tracked_items,
                    remaining_secs,
                }
            })
            .collect()
    }
}

/// Compute the target (fully-revealed) height for a toast based on its
/// body content.  Height = 2 (borders, title is on the top border) +
/// body lines, with a floor of `2 + min_interior_lines`.
fn compute_target_height(body: &str, min_interior_lines: u16) -> u16 {
    // Inner width is toast width minus 2 for borders.
    let inner_width = usize::from(TOAST_WIDTH.saturating_sub(2));
    let body_lines = if body.is_empty() {
        1
    } else {
        body.lines()
            .map(|line| wrapped_line_count(line, inner_width.max(1)))
            .sum::<usize>()
    };
    let raw = u16::try_from(2 + body_lines).unwrap_or(u16::MAX);
    raw.max(2 + min_interior_lines)
}

/// Estimate how many display rows a single body line needs when
/// rendered via `ratatui::widgets::Paragraph::wrap(Wrap { trim: false
/// })`, which word-wraps at whitespace. Char-level `div_ceil`
/// undercounts because word boundaries can force earlier breaks (e.g.
/// a 40-char word at the end of a 48-char inner width rolls onto a
/// new line), truncating the rendered body.
fn wrapped_line_count(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    if line.trim().is_empty() {
        return 1;
    }
    let mut rows = 0_usize;
    let mut current: usize = 0;
    for word in line.split_whitespace() {
        let word_width = unicode_width::UnicodeWidthStr::width(word);
        // Word wider than a row — it consumes (at least) its own rows.
        // Approximate: split across `ceil(word_width / width)` rows,
        // with the last fragment starting a fresh line.
        if word_width > width {
            if current > 0 {
                rows += 1;
            }
            let full = word_width / width;
            rows = rows.saturating_add(full);
            current = word_width % width;
            continue;
        }
        let need = if current == 0 {
            word_width
        } else {
            current + 1 + word_width
        };
        if need <= width {
            current = need;
        } else {
            rows += 1;
            current = word_width;
        }
    }
    if current > 0 {
        rows += 1;
    }
    rows.max(1)
}

// Phase 14 absorption: `ToastManager` impls `Pane` and `Hittable`
// directly. Pane render is a no-op (toasts render via the overlay
// path in `render.rs`); Hittable walks the recorded hit rects.

impl crate::tui::pane::Pane for ToastManager {
    fn render(
        &mut self,
        _frame: &mut ratatui::Frame<'_>,
        _area: ratatui::layout::Rect,
        _ctx: &crate::tui::pane::PaneRenderCtx<'_>,
    ) {
    }
}

impl crate::tui::pane::Hittable for ToastManager {
    fn hit_test_at(&self, pos: ratatui::layout::Position) -> Option<crate::tui::pane::HoverTarget> {
        for hit in &self.hits {
            if hit.close_rect.contains(pos) {
                return Some(crate::tui::pane::HoverTarget::Dismiss(
                    crate::tui::app::DismissTarget::Toast(hit.id),
                ));
            }
            if hit.card_rect.contains(pos) {
                return Some(crate::tui::pane::HoverTarget::ToastCard(hit.id));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Duration long enough for the full exit animation to complete.
    /// Use a generous upper bound since toast heights are now variable.
    const EXIT_ANIMATION: Duration = Duration::from_millis(20 * TOAST_LINE_REVEAL_MS + 1);

    #[test]
    fn wrapped_line_count_splits_at_word_boundaries() {
        // "foo bar" at width 5 fits on one line ("foo bar" = 7 doesn't
        // fit, so "foo" then "bar" = 2 rows).
        assert_eq!(wrapped_line_count("foo bar", 5), 2);
        // A single word wider than the row is split across multiple rows.
        assert_eq!(wrapped_line_count("aaaaaaaaaaaa", 5), 3);
        // An empty / whitespace-only line still reserves one row.
        assert_eq!(wrapped_line_count("", 10), 1);
        assert_eq!(wrapped_line_count("   ", 10), 1);
        // Content that fits exactly is one row.
        assert_eq!(wrapped_line_count("hello", 5), 1);
    }

    #[test]
    fn compute_target_height_uses_word_wrap_not_char_wrap() {
        // Body where word-wrap produces more rows than char-wrap. At
        // inner_width = TOAST_WIDTH - 2 the char-wrap estimator might
        // undercount when a long word rolls to a new line.
        let body = "short ".repeat(20);
        let height = compute_target_height(&body, 1);
        // Height >= 2 (borders) + at least 1 body row; the exact
        // number depends on TOAST_WIDTH, but the function must never
        // return less than the min-floor.
        assert!(height >= 3);
    }

    #[test]
    fn timed_toast_expires() {
        let mut manager = ToastManager::default();
        manager.push_timed("settings", "updated", Duration::from_millis(10), 1);
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
    fn task_toast_lingers_then_exits() {
        let mut manager = ToastManager::default();
        let linger = Duration::from_secs(1);
        let task = manager.push_task("cargo clean", "~/rust/bevy", 1);
        assert_eq!(manager.active(Instant::now()).len(), 1);

        manager.finish_task(task, linger);
        let now = Instant::now();
        manager.prune(now);
        // Still alive during linger period.
        assert_eq!(manager.active(now).len(), 1);

        // After linger, exit animation begins.
        let after_linger = now + linger + Duration::from_millis(10);
        manager.prune(after_linger);
        assert_eq!(manager.active(after_linger).len(), 1); // exit animation in progress

        // After linger + exit animation, toast is gone.
        let after_all = after_linger + EXIT_ANIMATION;
        manager.prune(after_all);
        assert!(manager.active(after_all).is_empty());
    }

    #[test]
    fn task_toast_body_can_be_updated() {
        let mut manager = ToastManager::default();
        let task = manager.push_task("startup git", "loading", 1);
        manager.update_task_body(task, "2 remaining");
        let active = manager.active(Instant::now());
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].body(), "2 remaining");
    }

    #[test]
    fn permanent_toast_stays_after_prune() {
        let mut manager = ToastManager::default();
        manager.push_persistent("error", "bad keymap", ToastStyle::Error, None, 1);

        // Prune many times — permanent toast stays.
        let later = Instant::now() + Duration::from_hours(1);
        manager.prune(later);
        assert_eq!(manager.active(later).len(), 1);
    }

    #[test]
    fn permanent_toast_dismissed_by_user() {
        let mut manager = ToastManager::default();
        let id = manager.push_persistent("error", "bad keymap", ToastStyle::Error, None, 1);
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
        manager.push_persistent("error", "bad", ToastStyle::Error, None, 1);
        let active = manager.active(Instant::now());
        assert_eq!(active[0].style(), ToastStyle::Error);
    }

    #[test]
    fn toast_view_exposes_action_path() {
        let mut manager = ToastManager::default();
        let path: AbsolutePath = "/tmp/keymap.toml".into();
        manager.push_persistent("error", "bad", ToastStyle::Error, Some(path.clone()), 1);
        let active = manager.active(Instant::now());
        assert_eq!(active[0].action_path(), Some(path.as_path()));
    }

    #[test]
    fn variable_height_short_body() {
        // 2 borders + 1 body line = 3, min_interior_lines=1 floor is 3
        assert_eq!(compute_target_height("short", 1), 3);
    }

    #[test]
    fn variable_height_long_body_no_clamp() {
        let body = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        // 2 + 10 = 12, no max clamp
        assert_eq!(compute_target_height(body, 1), 12);
    }

    #[test]
    fn variable_height_multiline_body() {
        let body = "line1\nline2\nline3";
        // 2 (borders) + 3 body lines = 5
        assert_eq!(compute_target_height(body, 1), 5);
    }

    #[test]
    fn min_interior_lines_raises_floor() {
        // Body has 1 line, but min_interior_lines=2 means floor is 4
        assert_eq!(compute_target_height("short", 2), 4);
    }

    #[test]
    fn min_interior_lines_does_not_shrink() {
        // Body has 5 lines, min_interior_lines=1 means floor is 3, actual is 7
        let body = "a\nb\nc\nd\ne";
        assert_eq!(compute_target_height(body, 1), 7);
    }
}

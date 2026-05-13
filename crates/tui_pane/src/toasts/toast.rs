use std::time::Duration;
use std::time::Instant;

use super::ToastBody;
use super::ToastId;
use super::ToastTaskId;
use super::ToastView;
use super::TrackedItem;
use super::TrackedItemView;
use super::toast_body_width;
use crate::AppContext;
use crate::ToastSettings;

/// Visual style applied to a toast card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastStyle {
    /// Default informational toast style.
    Normal,
    /// Warning toast style.
    Warning,
    /// Error toast style.
    Error,
}

/// Lifetime policy for a toast entry.
#[derive(Clone, Copy, Debug)]
pub enum ToastLifetime {
    /// Toast exits after the given instant.
    Timed {
        /// Instant when the toast should start exiting.
        timeout_at: Instant,
    },
    /// Toast follows a task lifecycle.
    Task {
        /// Associated task identifier.
        task_id: ToastTaskId,
        /// Current task status.
        status:  ToastTaskStatus,
    },
    /// Toast remains until explicitly dismissed.
    Persistent,
}

/// Runtime state for a task-backed toast.
#[derive(Clone, Copy, Debug)]
pub enum ToastTaskStatus {
    /// Task is still running.
    Running,
    /// Task has finished and remains visible for a linger duration.
    Finished {
        /// Instant when the task finished.
        finished_at: Instant,
        /// How long the finished toast remains live.
        linger:      Duration,
    },
}

/// Render phase for a toast entry.
#[derive(Clone, Copy, Debug)]
pub enum ToastPhase {
    /// Toast is fully visible.
    Visible,
    /// Toast is in its exit animation.
    Exiting {
        /// Instant when the exit animation started.
        started_at: Instant,
    },
}

/// Stored toast entry.
#[derive(Clone, Debug)]
pub struct Toast<Ctx: AppContext> {
    pub(super) id:                 ToastId,
    pub(super) title:              String,
    pub(super) body:               ToastBody,
    pub(super) style:              ToastStyle,
    pub(super) lifetime:           ToastLifetime,
    pub(super) phase:              ToastPhase,
    pub(super) action:             Option<Ctx::ToastAction>,
    pub(super) tracked_items:      Vec<TrackedItem>,
    pub(super) created_at:         Instant,
    pub(super) min_interior_lines: usize,
    pub(super) item_linger:        Duration,
}

impl<Ctx: AppContext> Toast<Ctx> {
    /// Return this toast's identifier.
    pub const fn id(&self) -> ToastId { self.id }

    /// Return this toast's title.
    pub fn title(&self) -> &str { &self.title }

    /// Return this toast's structured body.
    pub const fn body(&self) -> &ToastBody { &self.body }

    /// Return this toast's body as display text.
    pub fn body_text(&self) -> String { self.body.as_text() }

    /// Return this toast's style.
    pub const fn style(&self) -> ToastStyle { self.style }

    /// Return this toast's action payload, if any.
    pub const fn action(&self) -> Option<&Ctx::ToastAction> { self.action.as_ref() }

    pub(super) const fn task_id(&self) -> Option<ToastTaskId> {
        match self.lifetime {
            ToastLifetime::Task { task_id, .. } => Some(task_id),
            ToastLifetime::Timed { .. } | ToastLifetime::Persistent => None,
        }
    }

    pub(super) fn is_live(&self, now: Instant) -> bool {
        matches!(self.phase, ToastPhase::Visible) && !self.should_exit(now)
    }

    pub(super) fn is_renderable(&self, now: Instant, settings: &ToastSettings) -> bool {
        match self.phase {
            ToastPhase::Visible => !self.should_exit(now),
            ToastPhase::Exiting { started_at } => self.exit_lines(now, settings, started_at) > 0,
        }
    }

    pub(super) fn should_exit(&self, now: Instant) -> bool {
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

    pub(super) fn view(&self, now: Instant, settings: &ToastSettings) -> ToastView {
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
                u16::try_from(lines)
                    .unwrap_or(u16::MAX)
                    .min(target)
                    .max(self.min_height())
            },
            ToastPhase::Exiting { started_at } => self.exit_lines(now, settings, started_at),
        }
    }

    fn exit_lines(&self, now: Instant, settings: &ToastSettings, started_at: Instant) -> u16 {
        let target = self.target_height(settings);
        let elapsed = now.saturating_duration_since(started_at);
        let line_ms = settings.animation.exit_duration.get().as_millis().max(1);
        let hidden = u16::try_from(elapsed.as_millis() / line_ms).unwrap_or(u16::MAX);
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

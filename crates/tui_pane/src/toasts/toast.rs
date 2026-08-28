use std::time::Duration;
use std::time::Instant;

use super::ToastBody;
use super::ToastId;
use super::ToastTaskId;
use super::ToastView;
use super::TrackedItem;
use super::TrackedItemView;
use super::manager::ToastVisualDeadline;
use super::render::format::fade_level;
use super::toast_body_width;
use super::view::ToastActionState;
use crate::ACTIVITY_SPINNER;
use crate::AppContext;
use crate::ToastSettings;
use crate::constants::TOAST_ELAPSED_SECONDS_MILLIS;

/// Visual style applied to a toast card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastStyle {
    /// Default informational toast style.
    Normal,
    /// Success toast style — green, for a completed positive action.
    Success,
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

/// Whether a toast has line-height changes during its entrance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToastEntranceSchedule {
    /// The toast starts at its target height and has no entrance changes.
    Absent,
    /// The toast grows at line-height boundaries in this inclusive interval.
    Scheduled {
        /// First line-height boundary that changes the rendered toast.
        starts_at: Instant,
        /// Last line-height boundary that changes the rendered toast.
        ends_at:   Instant,
    },
}

/// Render phase for a toast entry.
#[derive(Clone, Copy, Debug)]
pub(super) enum ToastPhase {
    /// Toast is growing at line-height boundaries in this interval.
    Entering {
        /// First line-height boundary that changes the rendered toast.
        starts_at: Instant,
        /// Last line-height boundary that changes the rendered toast.
        ends_at:   Instant,
    },
    /// Toast has reached its target height.
    Static,
    /// Toast is in its exit animation.
    Exiting {
        /// Instant when the exit animation started.
        started_at: Instant,
    },
}

impl From<ToastEntranceSchedule> for ToastPhase {
    fn from(toast_entrance_schedule: ToastEntranceSchedule) -> Self {
        match toast_entrance_schedule {
            ToastEntranceSchedule::Absent => Self::Static,
            ToastEntranceSchedule::Scheduled { starts_at, ends_at } => {
                Self::Entering { starts_at, ends_at }
            },
        }
    }
}

/// Records whether the user has clicked the close affordance on a
/// toast.
///
/// Consulted by [`Toasts::reactivate_task`](crate::Toasts::reactivate_task)
/// so that a toast the user dismissed stays closed even while its
/// tracker keeps reporting new items — the underlying work
/// continues, only the UI surface stays gone for the rest of this
/// tracker session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastDismissal {
    /// Toast has not been dismissed by the user. The toast may
    /// still be auto-exiting via timer or completion path, but
    /// [`Toasts::reactivate_task`](crate::Toasts::reactivate_task)
    /// is free to bring it back when new work arrives.
    #[default]
    Open,
    /// User clicked the close affordance. Reactivation paths
    /// honor the close and leave the toast alone until prune
    /// removes it.
    ClosedByUser,
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
    pub(super) dismissal:          ToastDismissal,
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
        matches!(self.phase, ToastPhase::Entering { .. } | ToastPhase::Static)
            && !self.should_exit(now)
    }

    pub(super) fn is_renderable(&self, now: Instant, settings: &ToastSettings) -> bool {
        match self.phase {
            ToastPhase::Entering { .. } | ToastPhase::Static => !self.should_exit(now),
            // Task toasts skip the post-countdown exit animation:
            // the "Closing in N" countdown is itself the visual
            // closure signal, and the last tracked item's
            // individual linger ends at the same instant the
            // countdown reaches zero. Letting the Exiting phase
            // render past that point would leave the frame
            // visible without any countdown while items continue
            // to prune, which contradicts the deterministic
            // "countdown is the last thing on screen" contract.
            // Timed and Persistent toasts keep the exit animation
            // because they have no countdown of their own.
            ToastPhase::Exiting { .. } if matches!(self.lifetime, ToastLifetime::Task { .. }) => {
                false
            },
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

    pub(super) fn next_visual_change_deadline(
        &self,
        now: Instant,
        settings: &ToastSettings,
    ) -> ToastVisualDeadline {
        let rendered_content = self.next_rendered_content_deadline(now);
        match self.phase {
            ToastPhase::Entering { starts_at, ends_at } => next_line_height_boundary(
                now,
                starts_at,
                ends_at,
                animation_line_duration(settings.animation.entrance_duration.get()),
            )
            .earlier(self.expiry_deadline(now))
            .earlier(rendered_content),
            ToastPhase::Static => self.expiry_deadline(now).earlier(rendered_content),
            ToastPhase::Exiting { .. } if matches!(self.lifetime, ToastLifetime::Task { .. }) => {
                ToastVisualDeadline::NoVisualChangeScheduled
            },
            ToastPhase::Exiting { started_at } => {
                self.next_exit_visual_change_deadline(now, settings, started_at)
            },
        }
    }

    pub(super) fn view(&self, now: Instant, settings: &ToastSettings) -> ToastView {
        let min_height = self.min_height();
        let desired_height = self.current_visible_lines(now, settings).max(min_height);
        ToastView {
            id: self.id,
            title: self.title.clone(),
            body: self.body.as_text(),
            body_line_colors: self.body.line_colors(),
            style: self.style,
            action_state: ToastActionState::from(self.action.is_some()),
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
                            linger_fade_progress(
                                now.saturating_duration_since(completed_at),
                                self.item_linger,
                            )
                        })
                    });
                    TrackedItemView {
                        label: item.label.clone(),
                        linger_progress,
                        elapsed,
                        activity: item.activity,
                    }
                })
                .collect(),
            min_height,
            desired_height,
        }
    }

    fn min_height(&self) -> u16 { (self.min_interior_lines + 2).try_into().unwrap_or(u16::MAX) }

    /// Refresh a non-exiting toast's entrance phase from its wrapped target
    /// height.
    pub(super) fn refresh_entrance_phase(&mut self, settings: &ToastSettings) {
        if matches!(self.phase, ToastPhase::Exiting { .. }) {
            return;
        }
        self.phase = ToastPhase::from(self.entrance_schedule(settings));
    }

    fn entrance_schedule(&self, settings: &ToastSettings) -> ToastEntranceSchedule {
        let min_height = self.min_height();
        let target_height = self.target_height(settings);
        if target_height <= min_height {
            return ToastEntranceSchedule::Absent;
        }
        let line_duration = animation_line_duration(settings.animation.entrance_duration.get());
        ToastEntranceSchedule::Scheduled {
            starts_at: self.created_at + line_duration.saturating_mul(u32::from(min_height)),
            ends_at:   self.created_at
                + line_duration.saturating_mul(u32::from(target_height.saturating_sub(1))),
        }
    }

    fn current_visible_lines(&self, now: Instant, settings: &ToastSettings) -> u16 {
        let target = self.target_height(settings);
        match self.phase {
            ToastPhase::Entering { .. } => {
                let elapsed = now.saturating_duration_since(self.created_at);
                let line_ms =
                    animation_line_duration(settings.animation.entrance_duration.get()).as_millis();
                let lines = (elapsed.as_millis() / line_ms) + 1;
                u16::try_from(lines)
                    .unwrap_or(u16::MAX)
                    .min(target)
                    .max(self.min_height())
            },
            ToastPhase::Static => target,
            ToastPhase::Exiting { started_at } => self.exit_lines(now, settings, started_at),
        }
    }

    fn exit_lines(&self, now: Instant, settings: &ToastSettings, started_at: Instant) -> u16 {
        let target = self.target_height(settings);
        let elapsed = now.saturating_duration_since(started_at);
        let line_ms = animation_line_duration(settings.animation.exit_duration.get()).as_millis();
        let hidden = u16::try_from(elapsed.as_millis() / line_ms).unwrap_or(u16::MAX);
        target.saturating_sub(hidden)
    }

    fn next_exit_visual_change_deadline(
        &self,
        now: Instant,
        settings: &ToastSettings,
        started_at: Instant,
    ) -> ToastVisualDeadline {
        let line_duration = animation_line_duration(settings.animation.exit_duration.get());
        let target_height = self.target_height(settings);
        next_line_height_boundary(
            now,
            started_at + line_duration,
            started_at + line_duration.saturating_mul(u32::from(target_height)),
            line_duration,
        )
    }

    fn expiry_deadline(&self, now: Instant) -> ToastVisualDeadline {
        match self.lifetime {
            ToastLifetime::Timed { timeout_at } => future_deadline(now, timeout_at),
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => checked_future_deadline(now, finished_at, linger),
            ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => ToastVisualDeadline::NoVisualChangeScheduled,
        }
    }

    fn next_rendered_content_deadline(&self, now: Instant) -> ToastVisualDeadline {
        self.next_countdown_deadline(now)
            .earlier(self.next_whole_toast_linger_fade_deadline(now))
            .earlier(self.next_tracked_item_deadline(now))
    }

    fn next_whole_toast_linger_fade_deadline(&self, now: Instant) -> ToastVisualDeadline {
        match self.lifetime {
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => next_linger_fade_deadline(now, finished_at, linger),
            ToastLifetime::Timed { .. }
            | ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => ToastVisualDeadline::NoVisualChangeScheduled,
        }
    }

    fn next_countdown_deadline(&self, now: Instant) -> ToastVisualDeadline {
        let expires_at = match self.lifetime {
            ToastLifetime::Timed { timeout_at } => timeout_at,
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => {
                let Some(expires_at) = finished_at.checked_add(linger) else {
                    return ToastVisualDeadline::NoVisualChangeScheduled;
                };
                expires_at
            },
            ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => {
                return ToastVisualDeadline::NoVisualChangeScheduled;
            },
        };
        let Some(remaining) = expires_at.checked_duration_since(now) else {
            return ToastVisualDeadline::NoVisualChangeScheduled;
        };
        if remaining.is_zero() {
            return ToastVisualDeadline::NoVisualChangeScheduled;
        }
        let until_boundary = if remaining.subsec_nanos() == 0 {
            Duration::from_secs(1)
        } else {
            Duration::from_nanos(u64::from(remaining.subsec_nanos()))
        };
        checked_future_deadline(now, now, until_boundary)
    }

    fn next_tracked_item_deadline(&self, now: Instant) -> ToastVisualDeadline {
        self.tracked_items.iter().fold(
            ToastVisualDeadline::NoVisualChangeScheduled,
            |deadline, item| {
                let item_deadline = item.completed_at.map_or_else(
                    || {
                        item.started_at.map_or(
                            ToastVisualDeadline::NoVisualChangeScheduled,
                            |started_at| {
                                let elapsed = now.saturating_duration_since(started_at);
                                let spinner = checked_future_deadline(
                                    now,
                                    started_at,
                                    ACTIVITY_SPINNER.next_frame_boundary(elapsed),
                                );
                                let elapsed_readout = checked_future_deadline(
                                    now,
                                    started_at,
                                    next_elapsed_readout_boundary(elapsed),
                                );
                                spinner.earlier(elapsed_readout)
                            },
                        )
                    },
                    |completed_at| {
                        checked_future_deadline(now, completed_at, self.item_linger).earlier(
                            next_linger_fade_deadline(now, completed_at, self.item_linger),
                        )
                    },
                );
                deadline.earlier(item_deadline)
            },
        )
    }

    fn target_height(&self, settings: &ToastSettings) -> u16 {
        let width = toast_body_width(settings);
        let body_lines = self.body.wrapped_line_count(width);
        let item_lines = if self.tracked_items.is_empty() {
            body_lines
        } else {
            self.tracked_items.len()
        };
        let interior = self.min_interior_lines.max(item_lines);
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
                .map(whole_seconds_rounded_up),
            ToastLifetime::Task {
                status:
                    ToastTaskStatus::Finished {
                        finished_at,
                        linger,
                    },
                ..
            } => (finished_at + linger)
                .checked_duration_since(now)
                .map(whole_seconds_rounded_up),
            ToastLifetime::Task {
                status: ToastTaskStatus::Running,
                ..
            }
            | ToastLifetime::Persistent => None,
        }
    }
}

fn animation_line_duration(configured_duration: Duration) -> Duration {
    Duration::from_millis(u64::try_from(configured_duration.as_millis().max(1)).unwrap_or(u64::MAX))
}

fn next_linger_fade_deadline(
    now: Instant,
    linger_started_at: Instant,
    linger: Duration,
) -> ToastVisualDeadline {
    if linger.is_zero() {
        return ToastVisualDeadline::NoVisualChangeScheduled;
    }
    let elapsed = now.saturating_duration_since(linger_started_at);
    if elapsed >= linger {
        return ToastVisualDeadline::NoVisualChangeScheduled;
    }
    let current_level = fade_level(linger_fade_progress(elapsed, linger));
    let mut earliest_nanos = elapsed.as_nanos().saturating_add(1);
    let mut latest_nanos = linger.as_nanos();
    if fade_level(linger_fade_progress(
        duration_from_nanos(latest_nanos),
        linger,
    )) == current_level
    {
        return ToastVisualDeadline::NoVisualChangeScheduled;
    }
    while earliest_nanos < latest_nanos {
        let midpoint_nanos = earliest_nanos + (latest_nanos - earliest_nanos) / 2;
        let midpoint_level = fade_level(linger_fade_progress(
            duration_from_nanos(midpoint_nanos),
            linger,
        ));
        if midpoint_level == current_level {
            earliest_nanos = midpoint_nanos.saturating_add(1);
        } else {
            latest_nanos = midpoint_nanos;
        }
    }
    checked_future_deadline(now, linger_started_at, duration_from_nanos(earliest_nanos))
}

fn linger_fade_progress(elapsed: Duration, linger: Duration) -> f64 {
    elapsed.as_secs_f64() / linger.as_secs_f64()
}

fn duration_from_nanos(nanos: u128) -> Duration {
    let nanos_per_second = Duration::from_secs(1).as_nanos();
    let seconds = nanos / nanos_per_second;
    let subsecond_nanos = nanos % nanos_per_second;
    let Ok(seconds) = u64::try_from(seconds) else {
        return Duration::MAX;
    };
    Duration::new(seconds, u32::try_from(subsecond_nanos).unwrap_or(u32::MAX))
}

fn next_elapsed_readout_boundary(elapsed: Duration) -> Duration {
    if elapsed.as_millis() == 0 {
        return Duration::from_micros(
            u64::try_from(elapsed.as_micros().saturating_add(1)).unwrap_or(u64::MAX),
        );
    }
    if elapsed.as_millis() < TOAST_ELAPSED_SECONDS_MILLIS {
        return Duration::from_millis(
            u64::try_from(elapsed.as_millis().saturating_add(1)).unwrap_or(u64::MAX),
        );
    }
    Duration::from_secs(elapsed.as_secs().saturating_add(1))
}

fn whole_seconds_rounded_up(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

fn future_deadline(now: Instant, deadline: Instant) -> ToastVisualDeadline {
    if deadline > now {
        ToastVisualDeadline::At(deadline)
    } else {
        ToastVisualDeadline::NoVisualChangeScheduled
    }
}

fn checked_future_deadline(
    now: Instant,
    starts_at: Instant,
    elapsed: Duration,
) -> ToastVisualDeadline {
    starts_at
        .checked_add(elapsed)
        .map_or(ToastVisualDeadline::NoVisualChangeScheduled, |deadline| {
            future_deadline(now, deadline)
        })
}

fn next_line_height_boundary(
    now: Instant,
    starts_at: Instant,
    ends_at: Instant,
    line_duration: Duration,
) -> ToastVisualDeadline {
    if now < starts_at {
        return ToastVisualDeadline::At(starts_at);
    }
    if now >= ends_at {
        return ToastVisualDeadline::NoVisualChangeScheduled;
    }
    let completed_intervals =
        now.saturating_duration_since(starts_at).as_millis() / line_duration.as_millis();
    let next_interval = u32::try_from(completed_intervals.saturating_add(1)).unwrap_or(u32::MAX);
    ToastVisualDeadline::At((starts_at + line_duration.saturating_mul(next_interval)).min(ends_at))
}

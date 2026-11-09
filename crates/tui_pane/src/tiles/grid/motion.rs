use super::Duration;
#[cfg(test)]
use super::Instant;
use super::PROGRESS_SCALE;
use super::Rect;
use super::Transition;
/// The active motion's clock, compared without waiting in render tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tiles) enum MotionTiming {
    /// No motion is running.
    Settled,
    /// The start and duration that a content retarget keeps.
    Active { started: Instant, millis: u64 },
}

/// Whether cell geometry is settled or moving from a previous arrangement.
pub(super) enum GridMotion<Id> {
    /// The current layout is fully drawn at its destination.
    Settled,
    /// A timed move retains the layout it started from.
    Moving(Transition<Id>),
}

/// The clock policy used by grid motion.
#[derive(Clone, Copy, Default)]
pub(super) enum MotionClock {
    #[default]
    Elapsed,
    HeldAtHalf,
}

/// Whether a step's depth has seen the widths its destination draws at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DestinationDepthStatus {
    /// The depth came from demands measured for this arrangement.
    Resolved,
    /// A key used the preceding frame's demands and awaits its first render.
    AwaitingMeasuredWidths,
}

/// Interpolate each edge of `from` toward `to`.
pub(super) fn lerp_rect(from: Rect, to: Rect, progress: u32) -> Rect {
    let left = lerp(from.x, to.x, progress);
    let top = lerp(from.y, to.y, progress);
    let right = lerp(from.right(), to.right(), progress);
    let bottom = lerp(from.bottom(), to.bottom(), progress);
    Rect {
        x:      left,
        y:      top,
        width:  right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

/// `from` moved toward `to` by `progress`, on integers throughout.
pub(super) fn lerp(from: u16, to: u16, progress: u32) -> u16 {
    let (from, to) = (u32::from(from), u32::from(to));
    let value = if to >= from {
        from + (to - from) * progress / PROGRESS_SCALE
    } else {
        from - (from - to) * progress / PROGRESS_SCALE
    };
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Ease progress in and out, so a transition starts and lands gently
/// rather than stopping dead at both ends.
///
/// Half smoothstep, half linear. Smoothstep alone peaks at one and a
/// half times linear speed, and everything it borrows for that middle it
/// takes from the two ends -- which matters here because the grid moves
/// in whole cells. A tail that flat leaves the last cell of travel
/// waiting several frames longer than the ones before it, so the motion
/// reads as stopping and then jumping into place. Averaging with linear
/// pulls the peak down to one and a quarter, and the slowest step ends
/// up around twice the fastest rather than an order of magnitude.
pub(super) fn eased(progress: u32) -> u32 {
    let scale = u64::from(PROGRESS_SCALE);
    let progress = u64::from(progress.min(PROGRESS_SCALE));
    let smoothstep =
        (3 * progress * progress * scale - 2 * progress * progress * progress) / (scale * scale);
    u32::try_from(u64::midpoint(smoothstep, progress)).unwrap_or(PROGRESS_SCALE)
}

/// Grid travel and expanded travel share one progress calculation.
pub(super) fn travel_progress(elapsed: Duration, millis: u64) -> u32 {
    let elapsed = elapsed.as_millis();
    let total = u128::from(millis);
    if total == 0 || elapsed >= total {
        return PROGRESS_SCALE;
    }
    u32::try_from(elapsed * u128::from(PROGRESS_SCALE) / total).unwrap_or(PROGRESS_SCALE)
}

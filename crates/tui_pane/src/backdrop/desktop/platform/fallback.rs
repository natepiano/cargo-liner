//! The do-nothing capture backend for every platform but macOS.
//!
//! Nothing is captured and no window is described, so the animations
//! draw nothing rather than something taken from elsewhere.

use crate::backdrop::desktop::CaptureAttemptResult;
use crate::backdrop::desktop::CaptureAttemptSequence;
use crate::backdrop::desktop::CaptureFailure;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::candidate;

/// No capture backend outside macOS, so nothing is drawn.
pub(in crate::backdrop::desktop) const fn capture(
    _: Metrics,
    _: CaptureWindowTarget,
    sequence: CaptureAttemptSequence,
) -> CaptureAttemptResult {
    candidate::capture_failure_before_window_selection(
        sequence,
        CaptureFailure::UnsupportedPlatform,
    )
}

/// Nothing to ask, where there is no capture to ask about.
pub(in crate::backdrop::desktop) const fn window_frame(_: u32) -> Option<Frame> { None }

/// No windows to describe, so no title tells one from another.
pub(in crate::backdrop::desktop) const fn window_titles() -> Vec<(u32, Option<String>)> {
    Vec::new()
}

/// Nothing wears the marker where nothing can be asked.
pub(in crate::backdrop::desktop) const fn window_titled(_: &str) -> Option<u32> { None }

/// Nothing stands anywhere where there are no windows to describe.
pub(in crate::backdrop::desktop) const fn window_at(_: (f64, f64)) -> Option<u32> { None }

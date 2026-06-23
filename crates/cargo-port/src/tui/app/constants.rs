use std::time::Duration;

pub(super) const ANIMATION_TICK: Duration = Duration::from_millis(80);

// lint pause toasts
pub(super) const LINT_CANCELLED_TOAST_TITLE: &str = "Lints cancelled";
pub(super) const LINT_PAUSED_TOAST_BODY: &str = "Lint runs are paused.";
pub(super) const LINT_PAUSED_TOAST_TITLE: &str = "Lints paused";
pub(super) const LINT_RESUMED_TOAST_BODY: &str = "Catching up paused lint runs.";
pub(super) const LINT_RESUMED_TOAST_TITLE: &str = "Lints resumed";

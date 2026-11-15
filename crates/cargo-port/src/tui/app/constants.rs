use std::time::Duration;

pub(super) const ANIMATION_TICK: Duration = Duration::from_millis(80);

// `App::handle_crates_io_new_release` uses `CRATES_IO_NEW_RELEASE_TITLE` for new-version
// notifications.
pub(super) const CRATES_IO_NEW_RELEASE_TITLE: &str = "New release on crates.io";

// `App::refresh_project_storage_if_due` refreshes project volume space every
// `PROJECT_STORAGE_REFRESH_SECS`, even when other processes change disk usage.
/// Refresh the project volume's free capacity even when a non-project process
/// changes disk space.
pub(super) const PROJECT_STORAGE_REFRESH_SECS: u64 = 2;

// `LINT_PAUSED_TOAST_TITLE` and `LINT_PAUSED_TOAST_BODY` supply the pause toast;
// `LINT_CANCELLED_TOAST_TITLE` names cancellation, `LINT_PROJECT_PAUSED_TOAST_TITLE` and
// `LINT_PROJECT_RESUMED_TOAST_TITLE` name project transitions, and `LINT_RESUMED_TOAST_TITLE` with
// `LINT_RESUMED_TOAST_BODY` supplies the resume toast.
pub(super) const LINT_CANCELLED_TOAST_TITLE: &str = "Lints cancelled";
pub(super) const LINT_PAUSED_TOAST_BODY: &str = "Lint runs are paused.";
pub(super) const LINT_PAUSED_TOAST_TITLE: &str = "Lints paused";
pub(super) const LINT_PROJECT_PAUSED_TOAST_TITLE: &str = "Project lints paused";
pub(super) const LINT_PROJECT_RESUMED_TOAST_TITLE: &str = "Project lints resumed";
pub(super) const LINT_RESUMED_TOAST_BODY: &str = "Catching up paused lint runs.";
pub(super) const LINT_RESUMED_TOAST_TITLE: &str = "Lints resumed";

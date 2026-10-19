// `CATCH_UP_LINT_TOAST_TITLE` and `NORMAL_LINT_TOAST_TITLE` label lint-running toasts.
use std::time::Duration;
/// Title while the startup catch-up batch runs — projects re-linted once
/// startup finishes because their sources are newer than their last run.
pub(super) const CATCH_UP_LINT_TOAST_TITLE: &str = "Catch-up lints";
/// Title for the running-lint toast during normal file-triggered runs.
pub(super) const NORMAL_LINT_TOAST_TITLE: &str = "Lints";

// `OWNED_RUN_CHILD_REAP_POLL_INTERVAL` and the `UNACTIVATED_GROUP_EXIT_POLL_*` constants set
// owned-run process polling limits.
pub(super) const OWNED_RUN_CHILD_REAP_POLL_INTERVAL: Duration =
    std::time::Duration::from_millis(25);
pub(super) const UNACTIVATED_GROUP_EXIT_POLL_ATTEMPTS: usize = 100;
pub(super) const UNACTIVATED_GROUP_EXIT_POLL_INTERVAL: Duration =
    std::time::Duration::from_millis(10);

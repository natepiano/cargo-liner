// src tui state lint
use std::time::Duration;
/// Title while the startup catch-up batch runs — projects re-linted once
/// startup finishes because their sources are newer than their last run.
pub(super) const CATCH_UP_LINT_TOAST_TITLE: &str = "Catch-up lints";
/// Title for the running-lint toast during normal file-triggered runs.
pub(super) const NORMAL_LINT_TOAST_TITLE: &str = "Lints";

// owned run process actor
pub(super) const OWNED_RUN_CHILD_REAP_POLL_INTERVAL: Duration =
    std::time::Duration::from_millis(25);
pub(super) const UNACTIVATED_GROUP_EXIT_POLL_ATTEMPTS: usize = 100;
pub(super) const UNACTIVATED_GROUP_EXIT_POLL_INTERVAL: Duration =
    std::time::Duration::from_millis(10);

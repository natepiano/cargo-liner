// git

/// Branch-row value when git cannot read the checkout's HEAD.
pub(in crate::tui) const HEAD_UNREADABLE_LABEL: &str = "HEAD unreadable";

// project counts

pub(super) const PROJECT_LIBS_LABEL: &str = "lib";
pub(super) const PROJECT_MEMBERS_LABEL: &str = "members";
pub(super) const PROJECT_PROC_MACROS_LABEL: &str = "proc-macro";
pub(super) const PROJECT_SUBMODULES_LABEL: &str = "submodules";
pub(super) const PROJECT_VENDORED_LABEL: &str = "vendored";

// tests

pub(super) const TESTS_DOC_LABEL: &str = "doc";
pub(super) const TESTS_INTEGRATION_LABEL: &str = "integration";
pub(super) const TESTS_UNIT_LABEL: &str = "unit";

// src tui panes pane_data mod
/// Value shown in the crates.io `version` row when the project is
/// publishable but a confirmed crates.io outage means no data has landed
/// — the title already says "crates.io", so the cell only needs to say
/// the service is unreachable.
pub(in crate::tui::panes) const CRATES_IO_UNREACHABLE: &str = "unreachable";

pub(super) const CRATES_IO_VERSION_LABEL: &str = "Version";
pub(super) const CRATES_IO_DOWNLOADS_LABEL: &str = "Downloads";
pub(super) const CRATES_IO_ALPHA_LABEL: &str = "alpha";
pub(super) const CRATES_IO_BETA_LABEL: &str = "beta";
pub(super) const CRATES_IO_RC_LABEL: &str = "rc";
pub(super) const CRATES_IO_PRE_LABEL: &str = "pre";

/// Measure every release label before the release data arrives.
pub(in crate::tui::panes) const CRATES_IO_LABELS: [&str; 6] = [
    CRATES_IO_VERSION_LABEL,
    CRATES_IO_DOWNLOADS_LABEL,
    CRATES_IO_ALPHA_LABEL,
    CRATES_IO_BETA_LABEL,
    CRATES_IO_RC_LABEL,
    CRATES_IO_PRE_LABEL,
];

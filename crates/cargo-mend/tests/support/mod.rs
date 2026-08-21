#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]
#![allow(
    clippy::needless_raw_string_hashes,
    reason = "test fixtures use raw strings with varying hash counts for readability"
)]

mod diagnostics;
mod mend_json;
mod report;

pub(super) use std::collections::BTreeSet;
pub(super) use std::fs;
use std::path::Path;
use std::process::Command;

pub(super) use tempfile::tempdir;

pub(super) use self::diagnostics::AdvertisedFix;
pub(super) use self::diagnostics::DiagnosticCode;
pub(super) use self::diagnostics::FixSummaryBucket;
pub(super) use self::diagnostics::FixSupport;
pub(super) use self::diagnostics::diagnostic_spec;
pub(super) use self::mend_json::fix_support_for;
pub(super) use self::mend_json::mend_command;
pub(super) use self::mend_json::parse_mend_json_output;
pub(super) use self::report::ExpectedFinding;
pub(super) use self::report::Report;

/// Mirrors `src/config/pub_in_path.rs`. Fixtures pin the setting explicitly
/// because project `mend.toml` beats the machine-global config: a fixture that
/// writes none inherits whatever the developer configured, so its result would
/// depend on who runs the suite.
#[derive(Clone, Copy)]
pub(super) enum PubInPath {
    Permitted,
    Required,
}

impl PubInPath {
    pub(super) const fn config_value(self) -> &'static str {
        match self {
            Self::Permitted => "permitted",
            Self::Required => "required",
        }
    }
}

/// Writes the fixture's `mend.toml` pinning `pub_in_path`. A fixture that needs
/// more `mend.toml` content writes its own file afterwards and must carry the
/// pin in that literal.
pub(super) fn pin_pub_in_path(project_root: &Path, pub_in_path: PubInPath) {
    fs::write(
        project_root.join("mend.toml"),
        format!(
            "[visibility]\npub_in_path = \"{}\"\n",
            pub_in_path.config_value()
        ),
    )
    .expect("write fixture mend.toml");
}

pub(super) fn assert_summary_matches_findings(report: &Report) {
    mend_json::assert_summary_matches_findings(report);
}

pub(super) fn cargo_command() -> Command { mend_json::cargo_command() }

pub(super) fn expected_summary_from_findings(
    expected_findings: &[ExpectedFinding],
) -> self::report::Summary {
    mend_json::expected_summary_from_findings(expected_findings)
}

pub(super) fn expected_summary_text(report: &Report) -> String {
    mend_json::expected_summary_text(report)
}

pub(super) fn run_mend_json(manifest_path: &Path) -> Report {
    mend_json::run_mend_json(manifest_path)
}

pub(super) fn strip_ansi(input: &str) -> String { mend_json::strip_ansi(input) }

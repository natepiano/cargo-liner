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

pub(super) use self::diagnostics::DiagnosticCode;
pub(super) use self::diagnostics::FixSummaryBucket;
pub(super) use self::diagnostics::FixSupport;
pub(super) use self::diagnostics::diagnostic_spec;
pub(super) use self::mend_json::fix_support_for;
pub(super) use self::mend_json::mend_command;
pub(super) use self::mend_json::parse_mend_json_output;
pub(super) use self::report::ExpectedFinding;
pub(super) use self::report::Report;

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

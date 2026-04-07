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

mod helpers;
mod types;

pub(crate) use std::collections::BTreeSet;
pub(crate) use std::fs;

pub(crate) use tempfile::tempdir;

pub(crate) use self::cargo_mend_tests_support::DiagnosticCode;
pub(crate) use self::cargo_mend_tests_support::FixSupport;
pub(crate) use self::cargo_mend_tests_support::diagnostic_spec;
pub(crate) use self::helpers::assert_summary_matches_findings;
pub(crate) use self::helpers::cargo_command;
pub(crate) use self::helpers::expected_summary_from_findings;
pub(crate) use self::helpers::expected_summary_text;
pub(crate) use self::helpers::fix_support_for;
pub(crate) use self::helpers::mend_command;
pub(crate) use self::helpers::run_mend_json;
pub(crate) use self::helpers::strip_ansi;
pub(crate) use self::types::ExpectedFinding;
pub(crate) use self::types::Report;

pub(crate) mod cargo_mend_tests_support {
    #![allow(
        dead_code,
        reason = "include!() pulls in entire source files; only a subset is re-exported"
    )]

    mod config {
        include!("../../src/config.rs");
    }

    mod fix_support {
        include!("../../src/fix_support.rs");
    }

    mod diagnostics_impl {
        include!("../../src/diagnostics.rs");
    }

    pub(crate) use config::DiagnosticCode;
    pub(crate) use diagnostics_impl::diagnostic_spec;
    pub(crate) use fix_support::FixSummaryBucket;
    pub(crate) use fix_support::FixSupport;
}

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

pub use std::collections::BTreeSet;
pub use std::fs;

pub use tempfile::tempdir;

pub use self::cargo_mend_tests_support::DiagnosticCode;
pub use self::cargo_mend_tests_support::FixSupport;
pub use self::cargo_mend_tests_support::diagnostic_spec;
pub use self::helpers::assert_summary_matches_findings;
pub use self::helpers::cargo_command;
pub use self::helpers::expected_summary_from_findings;
pub use self::helpers::expected_summary_text;
pub use self::helpers::fix_support_for;
pub use self::helpers::mend_command;
pub use self::helpers::run_mend_json;
pub use self::helpers::strip_ansi;
pub use self::types::ExpectedFinding;
pub use self::types::Report;

pub mod cargo_mend_tests_support {
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

    pub use config::DiagnosticCode;
    pub use diagnostics_impl::diagnostic_spec;
    pub use fix_support::FixSummaryBucket;
    pub use fix_support::FixSupport;
}

#![allow(clippy::expect_used)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::too_many_lines)]

pub use std::collections::BTreeSet;
pub use std::fs;
pub use std::path::PathBuf;
use std::process::Command;

use regex::Regex;
use serde::Deserialize;
pub use tempfile::tempdir;

use self::cargo_mend_tests_support::FixSummaryBucket;
pub use self::cargo_mend_tests_support::FixSupport;
pub use self::cargo_mend_tests_support::diagnostic_specs;

pub fn clear_wrappers(command: &mut Command) -> &mut Command {
    command
        .env_remove("RUSTC")
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_BUILD_RUSTC_WRAPPER", "")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
}

pub fn cargo_command() -> Command {
    let mut command = Command::new("cargo");
    clear_wrappers(&mut command);
    command
}

pub fn mend_command() -> Command {
    let mut command = Command::new(mend_bin());
    clear_wrappers(&mut command);
    command
}

pub fn mend_bin() -> PathBuf { PathBuf::from(env!("CARGO_BIN_EXE_cargo-mend")) }

pub fn strip_ansi(input: &str) -> String {
    let ansi = Regex::new(r"\x1b\[[0-9;]*m").expect("compile ansi regex");
    ansi.replace_all(input, "").into_owned()
}

#[derive(Debug, Deserialize)]
pub struct Finding {
    pub code:        String,
    #[serde(default)]
    pub path:        String,
    #[serde(default)]
    pub item:        Option<String>,
    #[serde(default)]
    pub fix_support: FixSupport,
}

#[derive(Debug, Deserialize)]
pub struct Report {
    pub summary:  Summary,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
pub struct Summary {
    pub error_count:                    usize,
    pub warning_count:                  usize,
    pub fixable_with_fix_count:         usize,
    pub fixable_with_fix_pub_use_count: usize,
}

#[derive(Clone, Copy)]
pub struct ExpectedFinding<'a> {
    pub code:        &'a str,
    pub fix_support: FixSupport,
}

pub fn severity_for_code(code: &str) -> &'static str {
    match code {
        "forbidden_pub_crate" | "forbidden_pub_in_crate" | "review_pub_mod" => "error",
        _ => "warning",
    }
}

pub fn expected_summary(report: &Report) -> Summary {
    let mut summary = Summary {
        error_count:                    0,
        warning_count:                  0,
        fixable_with_fix_count:         0,
        fixable_with_fix_pub_use_count: 0,
    };

    for finding in &report.findings {
        match severity_for_code(&finding.code) {
            "error" => summary.error_count += 1,
            _ => summary.warning_count += 1,
        }

        let fix_support = if matches!(finding.fix_support, FixSupport::None) {
            diagnostic_specs()
                .iter()
                .find(|spec| spec.code == finding.code)
                .expect("known diagnostic code")
                .fix_support
        } else {
            finding.fix_support
        };
        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix_count += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use_count += 1,
            None => {},
        }
    }

    summary
}

pub fn assert_summary_matches_findings(report: &Report) {
    let expected = expected_summary(report);
    assert_eq!(report.summary.error_count, expected.error_count);
    assert_eq!(report.summary.warning_count, expected.warning_count);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected.fixable_with_fix_pub_use_count
    );
}

pub fn fix_support_for(code: &str, fix_support: FixSupport) -> FixSupport {
    if matches!(fix_support, FixSupport::None) {
        diagnostic_specs()
            .iter()
            .find(|spec| spec.code == code)
            .expect("known diagnostic code")
            .fix_support
    } else {
        fix_support
    }
}

pub fn expected_summary_from_findings(expected_findings: &[ExpectedFinding<'_>]) -> Summary {
    let mut summary = Summary {
        error_count:                    0,
        warning_count:                  0,
        fixable_with_fix_count:         0,
        fixable_with_fix_pub_use_count: 0,
    };

    for finding in expected_findings {
        match severity_for_code(finding.code) {
            "error" => summary.error_count += 1,
            _ => summary.warning_count += 1,
        }

        let fix_support = fix_support_for(finding.code, finding.fix_support);

        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix_count += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use_count += 1,
            None => {},
        }
    }

    summary
}

pub fn expected_summary_text(report: &Report) -> String {
    let mut parts = vec![
        format!("{} error(s)", report.summary.error_count),
        format!("{} warning(s)", report.summary.warning_count),
    ];

    if report.summary.fixable_with_fix_count > 0 {
        parts.push(format!(
            "{} fixable with `--fix`",
            report.summary.fixable_with_fix_count
        ));
    }

    if report.summary.fixable_with_fix_pub_use_count > 0 {
        parts.push(format!(
            "{} fixable with `--fix-pub-use`",
            report.summary.fixable_with_fix_pub_use_count
        ));
    }

    format!("summary: {}", parts.join(", "))
}

pub fn run_mend_json(manifest_path: &std::path::Path) -> Report {
    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--json")
        .output()
        .expect("run cargo-mend --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse mend json report")
}

pub mod cargo_mend_tests_support {
    #![allow(dead_code)]

    mod fix_support {
        include!("../../src/fix_support.rs");
    }

    mod diagnostics_impl {
        include!("../../src/diagnostics.rs");
    }

    pub use diagnostics_impl::*;
    pub use fix_support::FixSummaryBucket;
    pub use fix_support::FixSupport;

    pub const fn diagnostic_specs() -> &'static [DiagnosticSpec] { DIAGNOSTICS }
}

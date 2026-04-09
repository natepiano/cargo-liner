use std::path::PathBuf;
use std::process::Command;

use regex::Regex;

use super::DiagnosticCode;
use super::FixSummaryBucket;
use super::FixSupport;
use super::diagnostic_spec;
use super::types::ExpectedFinding;
use super::types::Report;
use super::types::Summary;

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

pub const fn severity_for_code(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::ForbiddenPubCrate
        | DiagnosticCode::ForbiddenPubInCrate
        | DiagnosticCode::ReviewPubMod => "error",
        _ => "warning",
    }
}

pub fn expected_summary(report: &Report) -> Summary {
    let mut summary = Summary {
        errors:                   0,
        warnings:                 0,
        fixable_with_fix:         0,
        fixable_with_fix_pub_use: 0,
    };

    for finding in &report.findings {
        match severity_for_code(finding.code) {
            "error" => summary.errors += 1,
            _ => summary.warnings += 1,
        }

        let fix_support = if matches!(finding.fix_support, FixSupport::None) {
            diagnostic_spec(finding.code).fix_support
        } else {
            finding.fix_support
        };
        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use += 1,
            None => {},
        }
    }

    summary
}

pub fn assert_summary_matches_findings(report: &Report) {
    let expected = expected_summary(report);
    assert_eq!(report.summary.errors, expected.errors);
    assert_eq!(report.summary.warnings, expected.warnings);
    assert_eq!(report.summary.fixable_with_fix, expected.fixable_with_fix);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected.fixable_with_fix_pub_use
    );
}

pub const fn fix_support_for(code: DiagnosticCode, fix_support: FixSupport) -> FixSupport {
    if matches!(fix_support, FixSupport::None) {
        diagnostic_spec(code).fix_support
    } else {
        fix_support
    }
}

pub fn expected_summary_from_findings(expected_findings: &[ExpectedFinding]) -> Summary {
    let mut summary = Summary {
        errors:                   0,
        warnings:                 0,
        fixable_with_fix:         0,
        fixable_with_fix_pub_use: 0,
    };

    for finding in expected_findings {
        match severity_for_code(finding.code) {
            "error" => summary.errors += 1,
            _ => summary.warnings += 1,
        }

        let fix_support = fix_support_for(finding.code, finding.fix_support);

        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use += 1,
            None => {},
        }
    }

    summary
}

/// Returns a substring that must appear in the rendered summary.
/// Checks the first summary row (mend errors/warnings).
pub fn expected_summary_text(report: &Report) -> String {
    if report.summary.errors > 0 {
        return format!(
            "{} {}",
            report.summary.errors,
            if report.summary.errors == 1 {
                "mend error"
            } else {
                "mend errors"
            }
        );
    }
    if report.summary.warnings > 0 {
        return format!(
            "{} {}",
            report.summary.warnings,
            if report.summary.warnings == 1 {
                "mend warning"
            } else {
                "mend warnings"
            }
        );
    }
    "no issues found".to_string()
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

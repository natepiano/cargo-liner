use std::path::PathBuf;
use std::process::Command;

use regex::Regex;
use serde_json::Value;

use super::DiagnosticCode;
use super::FixSummaryBucket;
use super::FixSupport;
use super::diagnostic_spec;
use super::types::ExpectedFinding;
use super::types::Report;
use super::types::Summary;

fn clear_wrappers(command: &mut Command) -> &mut Command {
    command
        .env_remove("RUSTC")
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_BUILD_RUSTC_WRAPPER", "")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
}

pub(super) fn cargo_command() -> Command {
    let mut command = Command::new("cargo");
    clear_wrappers(&mut command);
    command
}

pub fn mend_command() -> Command {
    let mut command = Command::new(mend_bin());
    clear_wrappers(&mut command);
    command
}

fn mend_bin() -> PathBuf { PathBuf::from(env!("CARGO_BIN_EXE_cargo-mend")) }

pub(super) fn strip_ansi(input: &str) -> String {
    let ansi = Regex::new(r"\x1b\[[0-9;]*m").expect("compile ansi regex");
    ansi.replace_all(input, "").into_owned()
}

const fn severity_for_code(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::ForbiddenPubCrate
        | DiagnosticCode::ForbiddenPubInCrate
        | DiagnosticCode::ReviewPubMod => "error",
        _ => "warning",
    }
}

fn expected_summary(report: &Report) -> Summary {
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

pub(super) fn assert_summary_matches_findings(report: &Report) {
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

pub(super) fn expected_summary_from_findings(expected_findings: &[ExpectedFinding]) -> Summary {
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
pub(super) fn expected_summary_text(report: &Report) -> String {
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

pub(super) fn run_mend_json(manifest_path: &std::path::Path) -> Report {
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
    parse_mend_json_output(&output.stdout)
}

pub fn parse_mend_json_output(stdout: &[u8]) -> Report {
    let output = std::str::from_utf8(stdout).expect("decode cargo-mend json output");
    let mut findings = Vec::new();
    let mut build_finished = false;

    for (line_number, line) in output.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let message: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!(
                "parse cargo JSON message on line {}: {error}\nline:\n{line}",
                line_number + 1
            )
        });
        match message.get("reason").and_then(Value::as_str) {
            Some("compiler-message") => findings.push(finding_from_compiler_message(&message)),
            Some("build-finished") => {
                assert!(
                    message.get("success").and_then(Value::as_bool).is_some(),
                    "build-finished message missing success: {message}"
                );
                build_finished = true;
            },
            Some(_) => {},
            None => panic!("cargo JSON message missing reason: {message}"),
        }
    }

    assert!(
        build_finished,
        "cargo-mend JSON output did not include build-finished"
    );

    let mut report = Report {
        summary: Summary {
            errors:                   0,
            warnings:                 0,
            fixable_with_fix:         0,
            fixable_with_fix_pub_use: 0,
        },
        findings,
    };
    report.summary = expected_summary(&report);
    report
}

fn finding_from_compiler_message(message: &Value) -> super::types::Finding {
    let diagnostic = message
        .get("message")
        .unwrap_or_else(|| panic!("compiler-message missing message: {message}"));
    let code = diagnostic
        .pointer("/code/code")
        .and_then(Value::as_str)
        .map_or_else(
            || panic!("compiler-message missing diagnostic code: {message}"),
            code_from_str,
        );
    let span = diagnostic
        .get("spans")
        .and_then(Value::as_array)
        .and_then(|spans| {
            spans
                .iter()
                .find(|span| span.get("is_primary").and_then(Value::as_bool) == Some(true))
                .or_else(|| spans.first())
        });

    super::types::Finding {
        code,
        path: span
            .and_then(|span| span.get("file_name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        item: span
            .and_then(|span| span.get("label"))
            .and_then(Value::as_str)
            .map(String::from),
        fix_support: fix_support_from_diagnostic_children(diagnostic),
    }
}

fn code_from_str(code: &str) -> DiagnosticCode {
    match code {
        "forbidden_pub_crate" => DiagnosticCode::ForbiddenPubCrate,
        "forbidden_pub_in_crate" => DiagnosticCode::ForbiddenPubInCrate,
        "review_pub_mod" => DiagnosticCode::ReviewPubMod,
        "suspicious_pub" => DiagnosticCode::SuspiciousPub,
        "prefer_module_import" => DiagnosticCode::PreferModuleImport,
        "inline_path_qualified_type" => DiagnosticCode::InlinePathQualifiedType,
        "shorten_local_crate_import" => DiagnosticCode::ShortenLocalCrateImport,
        "replace_deep_super_import" => DiagnosticCode::ReplaceDeepSuperImport,
        "wildcard_parent_pub_use" => DiagnosticCode::WildcardParentPubUse,
        "internal_parent_pub_use_facade" => DiagnosticCode::InternalParentPubUseFacade,
        "narrow_to_pub_crate" => DiagnosticCode::NarrowToPubCrate,
        _ => panic!("unknown diagnostic code in cargo JSON output: {code}"),
    }
}

fn fix_support_from_diagnostic_children(diagnostic: &Value) -> FixSupport {
    let child_messages = diagnostic
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|child| child.get("message").and_then(Value::as_str));

    for message in child_messages {
        if message.contains("`cargo mend --fix-pub-use`") {
            return FixSupport::FixPubUse;
        }
    }

    FixSupport::None
}

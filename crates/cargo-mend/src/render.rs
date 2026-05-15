use std::fmt::Write as _;
use std::time::Duration;

use crate::constants::ANSI_BOLD;
use crate::constants::ANSI_BOLD_BLUE;
use crate::constants::ANSI_BOLD_GREEN;
use crate::constants::ANSI_BOLD_RED;
use crate::constants::ANSI_BOLD_YELLOW;
use crate::constants::ANSI_DIM;
use crate::diagnostics;
use crate::diagnostics::Finding;
use crate::diagnostics::Report;
use crate::diagnostics::Severity;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ColorMode {
    Enabled,
    Disabled,
}

impl ColorMode {
    pub(crate) const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

pub(crate) struct CompilerStats {
    pub warnings: usize,
    pub fixable:  usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Findings {
    None,
    CompilerOnly,
    MendOnly,
    Both,
}

impl Findings {
    const fn classify(report: &Report, compiler_stats: &CompilerStats) -> Self {
        let has_mend = !report.findings.is_empty();
        let has_compiler = compiler_stats.warnings > 0;
        match (has_mend, has_compiler) {
            (false, false) => Self::None,
            (false, true) => Self::CompilerOnly,
            (true, false) => Self::MendOnly,
            (true, true) => Self::Both,
        }
    }
}

pub(crate) fn render_human_report(
    report: &Report,
    compiler_stats: &CompilerStats,
    color_mode: ColorMode,
) -> String {
    let findings = Findings::classify(report, compiler_stats);
    if findings == Findings::None {
        return "No findings.\n".to_string();
    }

    let mut output = String::new();
    if matches!(findings, Findings::MendOnly | Findings::Both) {
        for finding in &report.findings {
            render_finding(&mut output, finding, color_mode);
        }
    }

    if let Some(errors_block) = errors_block(report, color_mode) {
        output.push_str(&errors_block);
        output.push('\n');
    }
    output.push_str(&summary_line(report, compiler_stats, color_mode));
    output.push('\n');
    output
}

pub(crate) fn render_timing(
    total: Duration,
    check: Duration,
    mend: Duration,
    color_mode: ColorMode,
) -> String {
    format!(
        "    {} in {:.2}s (check: {:.2}s, mend: {:.2}s)",
        paint("Finished", ANSI_BOLD_GREEN, color_mode),
        total.as_secs_f64(),
        check.as_secs_f64(),
        mend.as_secs_f64(),
    )
}

fn render_finding(output: &mut String, finding: &Finding, color_mode: ColorMode) {
    let severity = severity_label(finding.severity, color_mode);
    let headline = diagnostics::finding_headline(finding);
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let arrow_pad = " ".repeat(gutter_width);
    let _ = writeln!(output, "{severity} {headline}");
    let _ = writeln!(
        output,
        "{arrow_pad}{} {}:{}:{}",
        blue_bold("-->", color_mode),
        finding.path,
        finding.line,
        finding.column
    );
    let _ = writeln!(output, "{gutter_pad}{}", blue_bold("|", color_mode));
    let _ = writeln!(
        output,
        "{:>width$} {} {}",
        blue_bold(&line_label, color_mode),
        blue_bold("|", color_mode),
        finding.source_line,
        width = gutter_width
    );
    let _ = writeln!(
        output,
        "{gutter_pad}{} {}",
        blue_bold("|", color_mode),
        severity_marker(
            finding.severity,
            finding.column,
            finding.highlight_len,
            color_mode
        )
    );
    if let Some(inline_help) = diagnostics::custom_inline_help_text(finding)
        .or_else(|| diagnostics::inline_help_text(finding))
    {
        let _ = writeln!(output, "{gutter_pad}{}", blue_bold("|", color_mode));
        let _ = writeln!(
            output,
            "{gutter_pad}{} {}",
            blue_bold("|", color_mode),
            blue_bold(&format!("help: {inline_help}"), color_mode)
        );
    }

    let reasons = diagnostics::detail_reasons(finding);
    if diagnostics::custom_inline_help_text(finding).is_some()
        || diagnostics::inline_help_text(finding).is_some()
        || !reasons.is_empty()
    {
        let _ = writeln!(output, "{gutter_pad}{}", blue_bold("|", color_mode));
    }
    if !reasons.is_empty() {
        for reason in reasons {
            let _ = writeln!(
                output,
                "{gutter_pad}{} {}",
                diagnostic_label("note", color_mode),
                reason
            );
        }
    }
    let help_url = diagnostics::finding_help_url(finding);
    let _ = writeln!(
        output,
        "{gutter_pad}{} for further information visit {help_url}",
        diagnostic_label("help", color_mode)
    );
    let _ = writeln!(output);
}

const fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

struct SummaryRow {
    count:       usize,
    description: String,
    /// One entry per applicable fix flag. The first renders inline with the
    /// row; later entries render as continuation lines under the same row.
    fixables:    Vec<SummaryFixable>,
}

struct SummaryFixable {
    count:   usize,
    command: &'static str,
}

fn errors_block(report: &Report, color_mode: ColorMode) -> Option<String> {
    if report.summary.errors == 0 {
        return None;
    }
    let n = report.summary.errors;
    let label = pluralize(n, "mend error", "mend errors");
    Some(format!(
        "{} {n} {label} (not auto-fixable; fix manually)",
        paint("errors:", ANSI_BOLD_RED, color_mode)
    ))
}

/// How many fix categories have at least one fixable item.
const fn fixable_category_count(report: &Report, compiler_stats: &CompilerStats) -> usize {
    let mut n = 0;
    if report.summary.fixable_with_fix > 0 {
        n += 1;
    }
    if report.summary.fixable_with_fix_pub_use > 0 {
        n += 1;
    }
    if compiler_stats.fixable > 0 {
        n += 1;
    }
    n
}

fn summary_line(report: &Report, compiler_stats: &CompilerStats, color_mode: ColorMode) -> String {
    let mut rows = Vec::new();
    let categories = fixable_category_count(report, compiler_stats);
    let total_fixable = report.summary.fixable_with_fix
        + report.summary.fixable_with_fix_pub_use
        + compiler_stats.fixable;

    if compiler_stats.warnings > 0 {
        let n = compiler_stats.warnings;
        let mut fixables = Vec::new();
        if compiler_stats.fixable > 0 {
            fixables.push(SummaryFixable {
                count:   compiler_stats.fixable,
                command: "cargo mend --fix-compiler",
            });
        }
        rows.push(SummaryRow {
            count: n,
            description: pluralize(n, "compiler warning", "compiler warnings").to_string(),
            fixables,
        });
    }
    if report.summary.warnings > 0 {
        let n = report.summary.warnings;
        let mut fixables = Vec::new();
        if report.summary.fixable_with_fix > 0 {
            fixables.push(SummaryFixable {
                count:   report.summary.fixable_with_fix,
                command: "cargo mend --fix",
            });
        }
        if report.summary.fixable_with_fix_pub_use > 0 {
            fixables.push(SummaryFixable {
                count:   report.summary.fixable_with_fix_pub_use,
                command: "cargo mend --fix-pub-use",
            });
        }
        rows.push(SummaryRow {
            count: n,
            description: pluralize(n, "mend warning", "mend warnings").to_string(),
            fixables,
        });
    }

    if rows.is_empty() {
        return format!("{} no issues found", dim("summary:", color_mode));
    }

    // When fixables span multiple flag categories, append a `--fix-all` entry
    // to the last warning row so the single-command convergent option is
    // always one click away.
    if categories > 1
        && let Some(last) = rows.last_mut()
    {
        last.fixables.push(SummaryFixable {
            count:   total_fixable,
            command: "cargo mend --fix-all",
        });
    }

    render_summary_rows(&rows, color_mode)
}

fn render_summary_rows(rows: &[SummaryRow], color_mode: ColorMode) -> String {
    let count_width = rows.iter().map(|r| digit_count(r.count)).max().unwrap_or(1);
    let desc_width = rows.iter().map(|r| r.description.len()).max().unwrap_or(0);
    let fixable_count_width = rows
        .iter()
        .flat_map(|r| r.fixables.iter())
        .map(|f| digit_count(f.count))
        .max()
        .unwrap_or(0);
    let prefix = dim("summary:", color_mode);
    let indent = " ".repeat("summary:".len());
    // Continuation indent fills the count + description columns so the dash
    // aligns with the inline fixable on the parent row.
    let cont_indent = format!("{indent} {:>count_width$} {:<desc_width$}", "", "");

    let mut result = String::new();
    let mut first = true;
    for (i, row) in rows.iter().enumerate() {
        let leader = if i == 0 { &prefix } else { &indent };
        let inline = row.fixables.first();
        let inline_part = inline.map_or_else(String::new, |f| {
            format!(
                " - {:>width$} fixable with `{}`",
                f.count,
                f.command,
                width = fixable_count_width
            )
        });
        if !first {
            result.push('\n');
        }
        first = false;
        let _ = write!(
            result,
            "{leader} {:>count_width$} {:<desc_width$}{inline_part}",
            row.count, row.description,
        );
        for f in row.fixables.iter().skip(1) {
            result.push('\n');
            let _ = write!(
                result,
                "{cont_indent} - {:>width$} fixable with `{}`",
                f.count,
                f.command,
                width = fixable_count_width
            );
        }
    }
    result
}

fn digit_count(n: usize) -> usize { n.to_string().len() }

fn severity_label(severity: Severity, color_mode: ColorMode) -> String {
    match severity {
        Severity::Error => paint("error:", ANSI_BOLD_RED, color_mode),
        Severity::Warning => paint("warning:", ANSI_BOLD_YELLOW, color_mode),
    }
}

fn dim(text: &str, color_mode: ColorMode) -> String { paint(text, ANSI_DIM, color_mode) }

fn blue_bold(text: &str, color_mode: ColorMode) -> String {
    paint(text, ANSI_BOLD_BLUE, color_mode)
}

fn severity_marker(
    severity: Severity,
    column: usize,
    highlight_len: usize,
    color_mode: ColorMode,
) -> String {
    let indent = " ".repeat(column.saturating_sub(1));
    let carets = "^".repeat(highlight_len.max(1));
    let code = match severity {
        Severity::Error => ANSI_BOLD_RED,
        Severity::Warning => ANSI_BOLD_YELLOW,
    };
    format!("{indent}{}", paint(&carets, code, color_mode))
}

fn diagnostic_label(kind: &str, color_mode: ColorMode) -> String {
    let prefix = blue_bold("=", color_mode);
    let label = match kind {
        "help" => paint("help", ANSI_BOLD, color_mode),
        "note" => paint("note", ANSI_BOLD, color_mode),
        other => other.to_string(),
    };
    format!("{prefix} {label}:")
}

fn paint(text: &str, code: &str, color_mode: ColorMode) -> String {
    match color_mode {
        ColorMode::Enabled => format!("\x1b[{code}m{text}\x1b[0m"),
        ColorMode::Disabled => text.to_string(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::ColorMode;
    use super::CompilerStats;
    use super::render_human_report;
    use crate::config::DiagnosticCode;
    use crate::diagnostics::Finding;
    use crate::diagnostics::Report;
    use crate::diagnostics::ReportSummary;
    use crate::diagnostics::Severity;
    use crate::fix_support::FixSupport;

    fn compiler_stats(warnings: usize, fixable: usize) -> CompilerStats {
        CompilerStats { warnings, fixable }
    }

    fn mend_warning_report() -> Report {
        Report {
            root: ".".to_string(),
            summary: ReportSummary {
                warnings: 1,
                fixable_with_fix: 1,
                ..ReportSummary::default()
            },
            findings: vec![Finding {
                severity:      Severity::Warning,
                code:          DiagnosticCode::NarrowToPubCrate,
                path:          "src/lib.rs".to_string(),
                line:          1,
                column:        1,
                highlight_len: 3,
                source_line:   "pub fn example() {}".to_string(),
                item:          Some("example".to_string()),
                message:       "example warning".to_string(),
                suggestion:    Some("pub(crate) fn example() {}".to_string()),
                fixability:    FixSupport::NarrowToPubCrate,
                related:       None,
            }],
            ..Report::default()
        }
    }

    #[test]
    fn render_human_report_prints_no_findings_when_empty() {
        let output = render_human_report(
            &Report::default(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert_eq!(output, "No findings.\n");
    }

    #[test]
    fn render_human_report_shows_summary_for_compiler_only_output() {
        let output = render_human_report(
            &Report::default(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        assert!(output.contains("summary: 3 compiler warnings"));
        assert!(!output.contains("warning:"));
    }

    #[test]
    fn render_human_report_shows_mend_summary_without_compiler_row() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(output.contains("warning:"));
        assert!(output.contains("summary: 1 mend warning"));
        assert!(!output.contains("compiler warning"));
    }

    #[test]
    fn render_human_report_shows_combined_summary_for_mend_and_compiler_findings() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        assert!(output.contains("summary: 3 compiler warnings"));
        assert!(output.contains("1 mend warning"));
        assert!(
            !output.contains("total warnings"),
            "total-warnings action row should not appear; --fix-all is suggested per row instead"
        );
    }

    #[test]
    fn render_human_report_aligns_summary_count_column_across_rows() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        // Compiler row gets `--fix-compiler`; mend row gets `--fix` plus the
        // continuation `--fix-all` line because two categories are fixable.
        assert!(
            output.contains(
                "summary: 3 compiler warnings - 1 fixable with `cargo mend --fix-compiler`\n"
            ),
            "compiler row missing/misaligned:\n{output}"
        );
        assert!(
            output.contains("         1 mend warning      - 1 fixable with `cargo mend --fix`\n"),
            "mend row missing/misaligned:\n{output}"
        );
        // The `--fix-all` continuation line aligns under the inline fixable
        // (its dash sits in the same column as the mend row's dash).
        assert!(
            output.contains("                            - 2 fixable with `cargo mend --fix-all`"),
            "--fix-all continuation row missing/misaligned:\n{output}"
        );
    }

    fn pub_use_warning_report() -> Report {
        Report {
            root: ".".to_string(),
            summary: ReportSummary {
                warnings: 1,
                fixable_with_fix_pub_use: 1,
                ..ReportSummary::default()
            },
            findings: vec![Finding {
                severity:      Severity::Warning,
                code:          DiagnosticCode::InternalParentPubUseFacade,
                path:          "src/lib.rs".to_string(),
                line:          1,
                column:        1,
                highlight_len: 3,
                source_line:   "pub use child::Foo;".to_string(),
                item:          None,
                message:       "example".to_string(),
                suggestion:    None,
                fixability:    FixSupport::PubUse,
                related:       None,
            }],
            ..Report::default()
        }
    }

    fn errors_only_report() -> Report {
        Report {
            root: ".".to_string(),
            summary: ReportSummary {
                errors: 3,
                ..ReportSummary::default()
            },
            findings: vec![Finding {
                severity:      Severity::Error,
                code:          DiagnosticCode::ForbiddenPubCrate,
                path:          "src/lib.rs".to_string(),
                line:          1,
                column:        1,
                highlight_len: 3,
                source_line:   "pub(crate) fn x() {}".to_string(),
                item:          Some("x".to_string()),
                message:       "forbidden".to_string(),
                suggestion:    None,
                fixability:    FixSupport::None,
                related:       None,
            }],
            ..Report::default()
        }
    }

    #[test]
    fn summary_never_emits_combined_fix_pub_use_string() {
        // Both mend and pub-use have fixables — should list each flag on its
        // own line plus `--fix-all`, never `cargo mend --fix --fix-pub-use`.
        let report = Report {
            summary: ReportSummary {
                warnings: 2,
                fixable_with_fix: 1,
                fixable_with_fix_pub_use: 1,
                ..ReportSummary::default()
            },
            findings: vec![
                mend_warning_report().findings[0].clone(),
                pub_use_warning_report().findings[0].clone(),
            ],
            ..Report::default()
        };
        let output = render_human_report(&report, &compiler_stats(0, 0), ColorMode::Disabled);

        assert!(
            !output.contains("--fix --fix-pub-use"),
            "combined flag string must never appear:\n{output}"
        );
        assert!(
            output.contains("`cargo mend --fix`"),
            "expected dedicated `--fix` line:\n{output}"
        );
        assert!(
            output.contains("`cargo mend --fix-pub-use`"),
            "expected dedicated `--fix-pub-use` line:\n{output}"
        );
        assert!(
            output.contains("`cargo mend --fix-all`"),
            "expected `--fix-all` continuation line:\n{output}"
        );
    }

    #[test]
    fn summary_lists_one_line_per_fix_flag_plus_fix_all_aggregate() {
        // Bug-report scenario: 213 mend warnings split 1-and-1 across `--fix`
        // and `--fix-pub-use`. Three lines required.
        let report = Report {
            summary: ReportSummary {
                warnings: 213,
                fixable_with_fix: 1,
                fixable_with_fix_pub_use: 1,
                ..ReportSummary::default()
            },
            findings: vec![
                mend_warning_report().findings[0].clone(),
                pub_use_warning_report().findings[0].clone(),
            ],
            ..Report::default()
        };
        let output = render_human_report(&report, &compiler_stats(0, 0), ColorMode::Disabled);

        let fix_idx = output
            .find("1 fixable with `cargo mend --fix`")
            .expect("missing --fix line");
        let pub_use_idx = output
            .find("1 fixable with `cargo mend --fix-pub-use`")
            .expect("missing --fix-pub-use line");
        let fix_all_idx = output
            .find("2 fixable with `cargo mend --fix-all`")
            .expect("missing --fix-all aggregate line");
        assert!(
            fix_idx < pub_use_idx && pub_use_idx < fix_all_idx,
            "expected order --fix → --fix-pub-use → --fix-all:\n{output}"
        );
    }

    #[test]
    fn summary_suggests_pub_use_alone_when_only_pub_use_is_fixable() {
        let output = render_human_report(
            &pub_use_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(output.contains("`cargo mend --fix-pub-use`"));
        // Single category → no --fix-all line.
        assert!(!output.contains("--fix-all"));
    }

    #[test]
    fn summary_emits_per_flag_lines_when_compiler_and_mend_are_fixable() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(2, 2),
            ColorMode::Disabled,
        );

        assert!(
            output.contains("`cargo mend --fix-compiler`"),
            "compiler row should still suggest --fix-compiler:\n{output}"
        );
        assert!(
            output.contains("`cargo mend --fix`"),
            "mend row should suggest --fix:\n{output}"
        );
        assert!(
            output.contains("`cargo mend --fix-all`"),
            "multi-category aggregate should appear:\n{output}"
        );
    }

    #[test]
    fn errors_render_in_their_own_block_above_summary() {
        let output = render_human_report(
            &errors_only_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        let errors_idx = output.find("errors:").expect("errors header should appear");
        let summary_idx = output.find("summary:");
        if let Some(s) = summary_idx {
            assert!(
                errors_idx < s,
                "errors block must precede summary block:\n{output}"
            );
        }
        assert!(
            output.contains("not auto-fixable"),
            "errors block must say errors are not auto-fixable:\n{output}"
        );
        // Errors must NEVER show up in the "X fixable" summary count.
        assert!(!output.contains("mend errors -"));
    }

    #[test]
    fn errors_block_omitted_when_no_errors_present() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(!output.contains("errors:"));
    }
}

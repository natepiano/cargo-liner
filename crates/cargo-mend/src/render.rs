use std::fmt::Write as _;
use std::time::Duration;

use super::constants::ANSI_BOLD;
use super::constants::ANSI_BOLD_BLUE;
use super::constants::ANSI_BOLD_GREEN;
use super::constants::ANSI_BOLD_RED;
use super::constants::ANSI_BOLD_YELLOW;
use super::constants::ANSI_DIM;
use super::diagnostics;
use super::diagnostics::Finding;
use super::diagnostics::Report;
use super::diagnostics::Severity;

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
    pub warning_count: usize,
    pub fixable_count: usize,
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
        let has_compiler = compiler_stats.warning_count > 0;
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
    color: ColorMode,
) -> String {
    let findings = Findings::classify(report, compiler_stats);
    if findings == Findings::None {
        return "No findings.\n".to_string();
    }

    let mut output = String::new();
    if matches!(findings, Findings::MendOnly | Findings::Both) {
        for finding in &report.findings {
            render_finding(&mut output, finding, color);
        }
    }

    output.push_str(&summary_line(report, compiler_stats, color));
    output.push('\n');
    output
}

pub(crate) fn render_timing(
    total: Duration,
    check: Duration,
    mend: Duration,
    color: ColorMode,
) -> String {
    format!(
        "    {} in {:.2}s (check: {:.2}s, mend: {:.2}s)",
        paint("Finished", ANSI_BOLD_GREEN, color),
        total.as_secs_f64(),
        check.as_secs_f64(),
        mend.as_secs_f64(),
    )
}

fn render_finding(output: &mut String, finding: &Finding, color: ColorMode) {
    let severity = severity_label(finding.severity, color);
    let headline = diagnostics::finding_headline(finding);
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let arrow_pad = " ".repeat(gutter_width);
    let _ = writeln!(output, "{severity} {headline}");
    let _ = writeln!(
        output,
        "{}{} {}:{}:{}",
        arrow_pad,
        blue_bold("-->", color),
        finding.path,
        finding.line,
        finding.column
    );
    let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
    let _ = writeln!(
        output,
        "{:>width$} {} {}",
        blue_bold(&line_label, color),
        blue_bold("|", color),
        finding.source_line,
        width = gutter_width
    );
    let _ = writeln!(
        output,
        "{}{} {}",
        gutter_pad,
        blue_bold("|", color),
        severity_marker(
            finding.severity,
            finding.column,
            finding.highlight_len,
            color
        )
    );
    if let Some(inline_help) = diagnostics::custom_inline_help_text(finding)
        .or_else(|| diagnostics::inline_help_text(finding))
    {
        let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
        let _ = writeln!(
            output,
            "{}{} {}",
            gutter_pad,
            blue_bold("|", color),
            blue_bold(&format!("help: {inline_help}"), color)
        );
    }

    let reasons = diagnostics::detail_reasons(finding);
    if diagnostics::custom_inline_help_text(finding).is_some()
        || diagnostics::inline_help_text(finding).is_some()
        || !reasons.is_empty()
    {
        let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
    }
    if !reasons.is_empty() {
        for reason in reasons {
            let _ = writeln!(
                output,
                "{}{} {}",
                gutter_pad,
                diagnostic_label("note", color),
                reason
            );
        }
    }
    let help_url = diagnostics::finding_help_url(finding);
    let _ = writeln!(
        output,
        "{}{} for further information visit {help_url}",
        gutter_pad,
        diagnostic_label("help", color)
    );
    let _ = writeln!(output);
}

const fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

struct SummaryRow {
    count:       usize,
    description: String,
    fixable:     Option<SummaryFixable>,
}

struct SummaryFixable {
    count:   usize,
    command: &'static str,
}

struct SummaryActionRow {
    count:         usize,
    description:   &'static str,
    fixable_count: usize,
    message:       &'static str,
    command:       &'static str,
}

fn summary_line(report: &Report, compiler_stats: &CompilerStats, color: ColorMode) -> String {
    let mut rows = Vec::new();
    let mend_fixable_count =
        report.summary.fixable_with_fix + report.summary.fixable_with_fix_pub_use;

    if report.summary.errors > 0 {
        let n = report.summary.errors;
        rows.push(SummaryRow {
            count:       n,
            description: pluralize(n, "mend error", "mend errors").to_string(),
            fixable:     None,
        });
    }
    if compiler_stats.warning_count > 0 {
        let n = compiler_stats.warning_count;
        rows.push(SummaryRow {
            count:       n,
            description: pluralize(n, "compiler warning", "compiler warnings").to_string(),
            fixable:     (compiler_stats.fixable_count > 0).then_some(SummaryFixable {
                count:   compiler_stats.fixable_count,
                command: "cargo mend --fix-compiler",
            }),
        });
    }
    if report.summary.warnings > 0 {
        let n = report.summary.warnings;
        let command = if report.summary.fixable_with_fix_pub_use > 0
            && report.summary.fixable_with_fix == 0
        {
            "cargo mend --fix-pub-use"
        } else if report.summary.fixable_with_fix > 0 && report.summary.fixable_with_fix_pub_use > 0
        {
            "cargo mend --fix --fix-pub-use"
        } else {
            "cargo mend --fix"
        };
        rows.push(SummaryRow {
            count:       n,
            description: pluralize(n, "mend warning", "mend warnings").to_string(),
            fixable:     (mend_fixable_count > 0).then_some(SummaryFixable {
                count: mend_fixable_count,
                command,
            }),
        });
    }

    if rows.is_empty() {
        return format!("{} no issues found", dim("summary:", color));
    }

    let total_warnings = report.summary.warnings + compiler_stats.warning_count;
    let total_fixable = mend_fixable_count + compiler_stats.fixable_count;
    let action_row =
        (mend_fixable_count > 0 && compiler_stats.fixable_count > 0).then_some(SummaryActionRow {
            count:         total_warnings,
            description:   "total warnings",
            fixable_count: total_fixable,
            message:       "fix both mend and compiler warnings",
            command:       "cargo mend --fix-all",
        });

    render_summary_rows(&rows, action_row.as_ref(), color)
}

fn render_summary_rows(
    rows: &[SummaryRow],
    action_row: Option<&SummaryActionRow>,
    color: ColorMode,
) -> String {
    let count_width = rows.iter().map(|r| digit_count(r.count)).max().unwrap_or(1);
    let desc_width = rows.iter().map(|r| r.description.len()).max().unwrap_or(0);
    let fixable_count_width = rows
        .iter()
        .filter_map(|r| r.fixable.as_ref())
        .map(|f| digit_count(f.count))
        .max()
        .unwrap_or(0);
    let desc_width = action_row.map_or(desc_width, |row| desc_width.max(row.description.len()));
    let fixable_count_width = action_row.map_or(fixable_count_width, |row| {
        fixable_count_width.max(digit_count(row.fixable_count))
    });
    let prefix = dim("summary:", color);
    let indent = " ".repeat("summary:".len());

    let mut result = String::new();
    for (i, row) in rows.iter().enumerate() {
        let leader = if i == 0 { &prefix } else { &indent };
        let fixable_part = row.fixable.as_ref().map_or_else(String::new, |f| {
            format!(
                " - {:>width$} fixable with `{}`",
                f.count,
                f.command,
                width = fixable_count_width
            )
        });
        let line = format!(
            "{leader} {:>cw$} {:<dw$}{fixable_part}",
            row.count,
            row.description,
            cw = count_width,
            dw = desc_width,
        );
        if i > 0 {
            result.push('\n');
        }
        result.push_str(&line);
    }
    if let Some(row) = action_row {
        if !result.is_empty() {
            result.push('\n');
        }
        let _ = write!(
            result,
            "{indent} {:>cw$} {:<dw$} - {:>fw$} fixable - {} with `{}`",
            row.count,
            row.description,
            row.fixable_count,
            row.message,
            row.command,
            cw = count_width,
            dw = desc_width,
            fw = fixable_count_width,
        );
    }
    result
}

fn digit_count(n: usize) -> usize { n.to_string().len() }

fn severity_label(severity: Severity, color: ColorMode) -> String {
    match severity {
        Severity::Error => paint("error:", ANSI_BOLD_RED, color),
        Severity::Warning => paint("warning:", ANSI_BOLD_YELLOW, color),
    }
}

fn dim(text: &str, color: ColorMode) -> String { paint(text, ANSI_DIM, color) }

fn blue_bold(text: &str, color: ColorMode) -> String { paint(text, ANSI_BOLD_BLUE, color) }

fn severity_marker(
    severity: Severity,
    column: usize,
    highlight_len: usize,
    color: ColorMode,
) -> String {
    let indent = " ".repeat(column.saturating_sub(1));
    let carets = "^".repeat(highlight_len.max(1));
    let code = match severity {
        Severity::Error => ANSI_BOLD_RED,
        Severity::Warning => ANSI_BOLD_YELLOW,
    };
    format!("{indent}{}", paint(&carets, code, color))
}

fn diagnostic_label(kind: &str, color: ColorMode) -> String {
    let prefix = blue_bold("=", color);
    let label = match kind {
        "help" => paint("help", ANSI_BOLD, color),
        "note" => paint("note", ANSI_BOLD, color),
        other => other.to_string(),
    };
    format!("{prefix} {label}:")
}

fn paint(text: &str, code: &str, color: ColorMode) -> String {
    match color {
        ColorMode::Enabled => format!("\x1b[{code}m{text}\x1b[0m"),
        ColorMode::Disabled => text.to_string(),
    }
}

#[cfg(test)]
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

    fn compiler_stats(warning_count: usize, fixable_count: usize) -> CompilerStats {
        CompilerStats {
            warning_count,
            fixable_count,
        }
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
        assert!(output.contains("4 total warnings"));
        assert!(output.contains(
            "- 2 fixable - fix both mend and compiler warnings with `cargo mend --fix-all`"
        ));
    }

    #[test]
    fn render_human_report_aligns_summary_count_column_across_rows() {
        let output = render_human_report(
            &mend_warning_report(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        assert!(output.contains(
            "summary: 3 compiler warnings - 1 fixable with `cargo mend --fix-compiler`\n         1 mend warning      - 1 fixable with `cargo mend --fix`\n         4 total warnings    - 2 fixable - fix both mend and compiler warnings with `cargo mend --fix-all`\n"
        ));
    }
}

use std::fmt::Write as _;
use std::time::Duration;

use super::constants::ANSI_BOLD;
use super::constants::ANSI_BOLD_BLUE;
use super::constants::ANSI_BOLD_GREEN;
use super::constants::ANSI_BOLD_RED;
use super::constants::ANSI_BOLD_YELLOW;
use super::constants::ANSI_DIM;
use super::diagnostics;
use super::diagnostics::CompilerDiagnosticPresence;
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

pub(crate) struct CompilerStats {
    pub warning_count: usize,
    pub fixable_count: usize,
}

pub(crate) fn render_human_report(
    report: &Report,
    compiler_stats: &CompilerStats,
    color: ColorMode,
) -> String {
    if report.findings.is_empty()
        && report.compiler_diagnostic_presence == CompilerDiagnosticPresence::None
    {
        return "No findings.\n".to_string();
    }
    if report.findings.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    for finding in &report.findings {
        render_finding(&mut output, finding, color);
    }

    let _ = writeln!(output, "{}", summary_line(report, compiler_stats, color));
    output
}

pub(crate) fn render_timing(
    total: Duration,
    check: Duration,
    driver: Duration,
    mend: Duration,
    color: ColorMode,
) -> String {
    if driver > Duration::ZERO {
        format!(
            "    {} in {:.2}s (check: {:.2}s, driver: {:.2}s, mend: {:.2}s)",
            paint("Finished", ANSI_BOLD_GREEN, color),
            total.as_secs_f64(),
            check.as_secs_f64(),
            driver.as_secs_f64(),
            mend.as_secs_f64(),
        )
    } else {
        format!(
            "    {} in {:.2}s (check: {:.2}s, mend: {:.2}s)",
            paint("Finished", ANSI_BOLD_GREEN, color),
            total.as_secs_f64(),
            check.as_secs_f64(),
            mend.as_secs_f64(),
        )
    }
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

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
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

fn summary_line(report: &Report, compiler_stats: &CompilerStats, color: ColorMode) -> String {
    let mut rows = Vec::new();

    if report.summary.errors > 0 {
        let n = report.summary.errors;
        rows.push(SummaryRow {
            count:       n,
            description: pluralize(n, "mend error", "mend errors").to_string(),
            fixable:     None,
        });
    }
    if report.summary.warnings > 0 {
        let n = report.summary.warnings;
        let fixable_count =
            report.summary.fixable_with_fix + report.summary.fixable_with_fix_pub_use;
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
            fixable:     (fixable_count > 0).then_some(SummaryFixable {
                count: fixable_count,
                command,
            }),
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

    if rows.is_empty() {
        return format!("{} no issues found", dim("summary:", color));
    }

    render_summary_rows(&rows, color)
}

fn render_summary_rows(rows: &[SummaryRow], color: ColorMode) -> String {
    let count_width = rows.iter().map(|r| digit_count(r.count)).max().unwrap_or(1);
    let desc_width = rows.iter().map(|r| r.description.len()).max().unwrap_or(0);
    let fixable_count_width = rows
        .iter()
        .filter_map(|r| r.fixable.as_ref())
        .map(|f| digit_count(f.count))
        .max()
        .unwrap_or(0);

    let prefix = dim("summary:", color);
    let indent = " ".repeat("summary: ".len());

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
    result
}

fn digit_count(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    ((n as f64).log10().floor() as usize) + 1
}

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

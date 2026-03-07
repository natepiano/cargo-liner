use std::fmt::Write as _;

use super::diagnostics;
use super::diagnostics::Finding;
use super::diagnostics::Report;
use super::diagnostics::Severity;

pub(super) fn render_human_report(report: &Report, color: bool) -> String {
    if report.findings.is_empty() {
        return "No findings.\n".to_string();
    }

    let mut output = String::new();
    for finding in &report.findings {
        render_finding(&mut output, finding, color);
    }

    let error_count = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warn_count = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let _ = writeln!(output, "{}", summary_line(error_count, warn_count, color));
    output
}

fn render_finding(output: &mut String, finding: &Finding, color: bool) {
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

fn summary_line(error_count: usize, warn_count: usize, color: bool) -> String {
    format!(
        "{} {} error(s), {} warning(s)",
        dim("summary:", color),
        error_count,
        warn_count
    )
}

fn severity_label(severity: Severity, color: bool) -> String {
    match severity {
        Severity::Error => paint("error:", "1;31", color),
        Severity::Warning => paint("warn:", "1;33", color),
    }
}

fn dim(text: &str, color: bool) -> String { paint(text, "2", color) }

fn blue_bold(text: &str, color: bool) -> String { paint(text, "1;34", color) }

fn severity_marker(severity: Severity, column: usize, highlight_len: usize, color: bool) -> String {
    let indent = " ".repeat(column.saturating_sub(1));
    let carets = "^".repeat(highlight_len.max(1));
    let code = match severity {
        Severity::Error => "1;31",
        Severity::Warning => "1;33",
    };
    format!("{indent}{}", paint(&carets, code, color))
}

fn diagnostic_label(kind: &str, color: bool) -> String {
    let prefix = blue_bold("=", color);
    let label = match kind {
        "help" => paint("help", "1", color),
        "note" => paint("note", "1", color),
        other => other.to_string(),
    };
    format!("{prefix} {label}:")
}

fn paint(text: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

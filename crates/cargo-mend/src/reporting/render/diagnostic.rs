use std::fmt::Write as _;

use super::ColorMode;
use super::color;
use crate::compiler::DIAGNOSTIC_SEVERITY_ERROR_PREFIX;
use crate::compiler::DIAGNOSTIC_SEVERITY_WARNING_PREFIX;
use crate::reporting::constants::ANSI_BOLD;
use crate::reporting::constants::ANSI_BOLD_RED;
use crate::reporting::constants::ANSI_BOLD_YELLOW;
use crate::reporting::constants::RUSTC_LEVEL_HELP;
use crate::reporting::constants::RUSTC_LEVEL_NOTE;
use crate::reporting::diagnostics;
use crate::reporting::diagnostics::Finding;
use crate::reporting::diagnostics::Severity;
use crate::reporting::diagnostics::SourceMarkerColor;

enum FindingAnnotations<'a> {
    LinkOnly,
    InlineHelp(&'a str),
    DetailReasons(Vec<String>),
    InlineHelpAndReasons {
        help:    &'a str,
        reasons: Vec<String>,
    },
}

impl<'a> From<&'a Finding> for FindingAnnotations<'a> {
    fn from(finding: &'a Finding) -> Self {
        Self::for_text(
            diagnostics::resolved_inline_help_text(finding),
            diagnostics::detail_reasons(finding),
        )
    }
}

impl<'a> FindingAnnotations<'a> {
    fn for_text(help: Option<&'a str>, reasons: Vec<String>) -> Self {
        match (help, reasons.is_empty()) {
            (None, true) => Self::LinkOnly,
            (Some(help), true) => Self::InlineHelp(help),
            (None, false) => Self::DetailReasons(reasons),
            (Some(help), false) => Self::InlineHelpAndReasons { help, reasons },
        }
    }

    fn render(self, output: &mut String, gutter_pad: &str, color_mode: ColorMode) {
        match self {
            Self::LinkOnly => {},
            Self::InlineHelp(help) => {
                render_inline_help(output, help, gutter_pad, color_mode);
                render_separator(output, gutter_pad, color_mode);
            },
            Self::DetailReasons(reasons) => {
                render_separator(output, gutter_pad, color_mode);
                render_reasons(output, &reasons, gutter_pad, color_mode);
            },
            Self::InlineHelpAndReasons { help, reasons } => {
                render_inline_help(output, help, gutter_pad, color_mode);
                render_separator(output, gutter_pad, color_mode);
                render_reasons(output, &reasons, gutter_pad, color_mode);
            },
        }
    }
}

fn render_separator(output: &mut String, gutter_pad: &str, color_mode: ColorMode) {
    let _ = writeln!(output, "{gutter_pad}{}", color::blue_bold("|", color_mode));
}

fn render_inline_help(output: &mut String, help: &str, gutter_pad: &str, color_mode: ColorMode) {
    render_separator(output, gutter_pad, color_mode);
    let _ = writeln!(
        output,
        "{gutter_pad}{} {}",
        color::blue_bold("|", color_mode),
        color::blue_bold(&format!("help: {help}"), color_mode)
    );
}

fn render_reasons(
    output: &mut String,
    reasons: &[String],
    gutter_pad: &str,
    color_mode: ColorMode,
) {
    for reason in reasons {
        let _ = writeln!(
            output,
            "{gutter_pad}{} {}",
            diagnostic_label(RUSTC_LEVEL_NOTE, color_mode),
            reason
        );
    }
}

pub(super) fn render_finding(output: &mut String, finding: &Finding, color_mode: ColorMode) {
    let severity = severity_label(finding.severity(), color_mode);
    let headline = diagnostics::finding_headline(finding);
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let arrow_pad = " ".repeat(gutter_width);
    let _ = writeln!(output, "{severity} {headline}");
    let _ = writeln!(
        output,
        "{arrow_pad}{} {}:{}:{}",
        color::blue_bold("-->", color_mode),
        finding.path,
        finding.line,
        finding.column
    );
    let _ = writeln!(output, "{gutter_pad}{}", color::blue_bold("|", color_mode));
    let _ = writeln!(
        output,
        "{:>width$} {} {}",
        color::blue_bold(&line_label, color_mode),
        color::blue_bold("|", color_mode),
        finding.source_line,
        width = gutter_width
    );
    let _ = writeln!(
        output,
        "{gutter_pad}{} {}",
        color::blue_bold("|", color_mode),
        source_marker(
            finding.source_marker_color(),
            finding.column,
            finding.highlight_len,
            color_mode
        )
    );
    FindingAnnotations::from(finding).render(output, &gutter_pad, color_mode);
    let help_url = diagnostics::finding_help_url(finding);
    let _ = writeln!(
        output,
        "{gutter_pad}{} for further information visit {help_url}",
        diagnostic_label(RUSTC_LEVEL_HELP, color_mode)
    );
    let _ = writeln!(output);
}

fn severity_label(severity: Severity, color_mode: ColorMode) -> String {
    match severity {
        Severity::Error => {
            color::paint(DIAGNOSTIC_SEVERITY_ERROR_PREFIX, ANSI_BOLD_RED, color_mode)
        },
        Severity::Warning => color::paint(
            DIAGNOSTIC_SEVERITY_WARNING_PREFIX,
            ANSI_BOLD_YELLOW,
            color_mode,
        ),
    }
}

fn source_marker(
    marker_color: SourceMarkerColor,
    column: usize,
    highlight_len: usize,
    color_mode: ColorMode,
) -> String {
    let indent = " ".repeat(column.saturating_sub(1));
    let carets = "^".repeat(highlight_len.max(1));
    let code = match marker_color {
        SourceMarkerColor::Red => ANSI_BOLD_RED,
        SourceMarkerColor::Yellow => ANSI_BOLD_YELLOW,
    };
    format!("{indent}{}", color::paint(&carets, code, color_mode))
}

fn diagnostic_label(kind: &str, color_mode: ColorMode) -> String {
    let prefix = color::blue_bold("=", color_mode);
    let label = match kind {
        RUSTC_LEVEL_HELP => color::paint(RUSTC_LEVEL_HELP, ANSI_BOLD, color_mode),
        RUSTC_LEVEL_NOTE => color::paint(RUSTC_LEVEL_NOTE, ANSI_BOLD, color_mode),
        other => other.to_string(),
    };
    format!("{prefix} {label}:")
}

#[cfg(test)]
mod tests {
    use super::FindingAnnotations;
    use crate::reporting::render::ColorMode;

    #[test]
    fn automatic_headlines_keep_their_origin_marker_color() -> anyhow::Result<()> {
        for (origin, marker) in [
            ("error", "\x1b[1;31m^^^\x1b[0m"),
            ("warning", "\x1b[1;33m^^^\x1b[0m"),
        ] {
            let finding = serde_json::from_value(serde_json::json!({
                "severity": origin, "diagnostic_code": "suspicious_pub",
                "fixability": "restricted_annotation", "path": "src/lib.rs",
                "line": 1, "column": 1, "highlight_len": 3,
                "source_line": "pub fn item() {}", "message": "message",
            }))?;
            let mut output = String::new();
            super::render_finding(&mut output, &finding, ColorMode::Enabled);
            assert!(output.starts_with("\x1b[1;33mwarning:"));
            assert!(output.contains(marker));
        }
        Ok(())
    }

    #[test]
    fn annotations_preserve_separators_for_all_four_layouts() {
        for (help, reasons, expected) in [
            (None, vec![], ""),
            (Some("replace"), vec![], "  |\n  | help: replace\n  |\n"),
            (None, vec!["reason".to_string()], "  |\n  = note: reason\n"),
            (
                Some("replace"),
                vec!["first".to_string(), "second".to_string()],
                "  |\n  | help: replace\n  |\n  = note: first\n  = note: second\n",
            ),
            (Some(""), vec![], "  |\n  | help: \n  |\n"),
        ] {
            let mut output = String::new();
            FindingAnnotations::for_text(help, reasons).render(
                &mut output,
                "  ",
                ColorMode::Disabled,
            );
            assert_eq!(output, expected);
        }
    }
}

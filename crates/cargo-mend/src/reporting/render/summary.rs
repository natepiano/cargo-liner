use std::fmt::Write as _;

use super::ColorMode;
use super::CompilerStats;
use super::color;
use crate::reporting::constants::ANSI_BOLD_RED;
use crate::reporting::constants::CARGO_MEND_FIX;
use crate::reporting::constants::CARGO_MEND_FIX_ALL;
use crate::reporting::constants::CARGO_MEND_FIX_COMPILER;
use crate::reporting::constants::CARGO_MEND_FIX_PUB_USE;
use crate::reporting::constants::SUMMARY_LABEL;
use crate::reporting::diagnostics::Report;

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

const fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

pub(super) fn errors_block(report: &Report, color_mode: ColorMode) -> Option<String> {
    if report.summary.errors == 0 {
        return None;
    }
    let n = report.summary.errors;
    let label = pluralize(n, "mend error", "mend errors");
    Some(format!(
        "{} {n} {label} (not auto-fixable; fix manually)",
        color::paint("errors:", ANSI_BOLD_RED, color_mode)
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

pub(super) fn summary_line(
    report: &Report,
    compiler_stats: &CompilerStats,
    color_mode: ColorMode,
) -> String {
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
                command: CARGO_MEND_FIX_COMPILER,
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
                command: CARGO_MEND_FIX,
            });
        }
        if report.summary.fixable_with_fix_pub_use > 0 {
            fixables.push(SummaryFixable {
                count:   report.summary.fixable_with_fix_pub_use,
                command: CARGO_MEND_FIX_PUB_USE,
            });
        }
        rows.push(SummaryRow {
            count: n,
            description: pluralize(n, "mend warning", "mend warnings").to_string(),
            fixables,
        });
    }

    if rows.is_empty() {
        return format!("{} no issues found", color::dim(SUMMARY_LABEL, color_mode));
    }

    // When fixables span multiple flag categories, append a `--fix-all` entry
    // to the last warning row so the single-command convergent option is
    // always one click away.
    if categories > 1
        && let Some(last) = rows.last_mut()
    {
        last.fixables.push(SummaryFixable {
            count:   total_fixable,
            command: CARGO_MEND_FIX_ALL,
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
    let prefix = color::dim(SUMMARY_LABEL, color_mode);
    let indent = " ".repeat(SUMMARY_LABEL.len());
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crate::config::DiagnosticCode;
    use crate::reporting;
    use crate::reporting::ColorMode;
    use crate::reporting::CompilerStats;
    use crate::reporting::constants::CARGO_MEND_FIX;
    use crate::reporting::constants::CARGO_MEND_FIX_ALL;
    use crate::reporting::constants::CARGO_MEND_FIX_COMPILER;
    use crate::reporting::constants::CARGO_MEND_FIX_PUB_USE;
    use crate::reporting::constants::SUMMARY_LABEL;
    use crate::reporting::diagnostics::Finding;
    use crate::reporting::diagnostics::FixSupport;
    use crate::reporting::diagnostics::Report;
    use crate::reporting::diagnostics::ReportSummary;
    use crate::reporting::diagnostics::Severity;

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
                severity:        Severity::Warning,
                diagnostic_code: DiagnosticCode::NarrowToPubCrate,
                path:            "src/lib.rs".to_string(),
                line:            1,
                column:          1,
                highlight_len:   3,
                source_line:     "pub fn example() {}".to_string(),
                item:            Some("example".to_string()),
                message:         "example warning".to_string(),
                suggestion:      Some("pub(crate) fn example() {}".to_string()),
                fix_support:     FixSupport::NarrowToPubCrate,
                related:         None,
            }],
            ..Report::default()
        }
    }

    #[test]
    fn render_human_report_prints_no_findings_when_empty() {
        let output = reporting::render_human_report(
            &Report::default(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert_eq!(output, "No findings.\n");
    }

    #[test]
    fn render_human_report_shows_summary_for_compiler_only_output() {
        let output = reporting::render_human_report(
            &Report::default(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        assert!(output.contains(&format!("{SUMMARY_LABEL} 3 compiler warnings")));
        assert!(!output.contains("warning:"));
    }

    #[test]
    fn render_human_report_shows_mend_summary_without_compiler_row() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(output.contains("warning:"));
        assert!(output.contains(&format!("{SUMMARY_LABEL} 1 mend warning")));
        assert!(!output.contains("compiler warning"));
    }

    #[test]
    fn render_human_report_shows_combined_summary_for_mend_and_compiler_findings() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        assert!(output.contains(&format!("{SUMMARY_LABEL} 3 compiler warnings")));
        assert!(output.contains("1 mend warning"));
        assert!(
            !output.contains("total warnings"),
            "total-warnings action row should not appear; --fix-all is suggested per row instead"
        );
    }

    #[test]
    fn render_human_report_aligns_summary_count_column_across_rows() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(3, 1),
            ColorMode::Disabled,
        );

        // Compiler row gets `--fix-compiler`; mend row gets `--fix` plus the
        // continuation `--fix-all` line because two categories are fixable.
        assert!(
            output.contains(&format!(
                "{SUMMARY_LABEL} 3 compiler warnings - 1 fixable with `{CARGO_MEND_FIX_COMPILER}`\n"
            )),
            "compiler row missing/misaligned:\n{output}"
        );
        assert!(
            output.contains(&format!(
                "{} 1 mend warning      - 1 fixable with `{CARGO_MEND_FIX}`\n",
                " ".repeat(SUMMARY_LABEL.len())
            )),
            "mend row missing/misaligned:\n{output}"
        );
        // The `--fix-all` continuation line aligns under the inline fixable
        // (its dash sits in the same column as the mend row's dash).
        let mend_row_dash = output
            .lines()
            .find(|line| line.contains("mend warning"))
            .and_then(|line| line.find(" - "))
            .expect("mend row missing dash");
        let fix_all_row_dash = output
            .lines()
            .find(|line| line.contains(CARGO_MEND_FIX_ALL))
            .and_then(|line| line.find(" - "))
            .expect("--fix-all continuation row missing dash");
        assert_eq!(
            fix_all_row_dash, mend_row_dash,
            "--fix-all continuation row misaligned:\n{output}"
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
                severity:        Severity::Warning,
                diagnostic_code: DiagnosticCode::InternalParentPubUseFacade,
                path:            "src/lib.rs".to_string(),
                line:            1,
                column:          1,
                highlight_len:   3,
                source_line:     "pub use child::Foo;".to_string(),
                item:            None,
                message:         "example".to_string(),
                suggestion:      None,
                fix_support:     FixSupport::PubUse,
                related:         None,
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
                severity:        Severity::Error,
                diagnostic_code: DiagnosticCode::ForbiddenPubCrate,
                path:            "src/lib.rs".to_string(),
                line:            1,
                column:          1,
                highlight_len:   3,
                source_line:     "pub(crate) fn x() {}".to_string(),
                item:            Some("x".to_string()),
                message:         "forbidden".to_string(),
                suggestion:      None,
                fix_support:     FixSupport::None,
                related:         None,
            }],
            ..Report::default()
        }
    }

    #[test]
    fn summary_never_emits_combined_fix_pub_use_string() {
        // Both mend and pub-use have fixables; each flag should render on its
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
        let output =
            reporting::render_human_report(&report, &compiler_stats(0, 0), ColorMode::Disabled);

        assert!(
            !output.contains("--fix --fix-pub-use"),
            "combined flag string must never appear:\n{output}"
        );
        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX}`")),
            "expected dedicated `--fix` line:\n{output}"
        );
        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX_PUB_USE}`")),
            "expected dedicated `--fix-pub-use` line:\n{output}"
        );
        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX_ALL}`")),
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
        let output =
            reporting::render_human_report(&report, &compiler_stats(0, 0), ColorMode::Disabled);

        let fix_idx = output
            .find(&format!("1 fixable with `{CARGO_MEND_FIX}`"))
            .expect("missing --fix line");
        let pub_use_idx = output
            .find(&format!("1 fixable with `{CARGO_MEND_FIX_PUB_USE}`"))
            .expect("missing --fix-pub-use line");
        let fix_all_idx = output
            .find(&format!("2 fixable with `{CARGO_MEND_FIX_ALL}`"))
            .expect("missing --fix-all aggregate line");
        assert!(
            fix_idx < pub_use_idx && pub_use_idx < fix_all_idx,
            "expected order --fix -> --fix-pub-use -> --fix-all:\n{output}"
        );
    }

    #[test]
    fn summary_suggests_pub_use_alone_when_only_pub_use_is_fixable() {
        let output = reporting::render_human_report(
            &pub_use_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(output.contains(&format!("`{CARGO_MEND_FIX_PUB_USE}`")));
        // Single category means no `--fix-all` line.
        assert!(!output.contains("--fix-all"));
    }

    #[test]
    fn summary_emits_per_flag_lines_when_compiler_and_mend_are_fixable() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(2, 2),
            ColorMode::Disabled,
        );

        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX_COMPILER}`")),
            "compiler row should still suggest --fix-compiler:\n{output}"
        );
        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX}`")),
            "mend row should suggest --fix:\n{output}"
        );
        assert!(
            output.contains(&format!("`{CARGO_MEND_FIX_ALL}`")),
            "multi-category aggregate should appear:\n{output}"
        );
    }

    #[test]
    fn errors_render_in_their_own_block_above_summary() {
        let output = reporting::render_human_report(
            &errors_only_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        let errors_idx = output.find("errors:").expect("errors header should appear");
        let summary_idx = output.find(SUMMARY_LABEL);
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
        // Errors must never show up in the "X fixable" summary count.
        assert!(!output.contains("mend errors -"));
    }

    #[test]
    fn errors_block_omitted_when_no_errors_present() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(!output.contains("errors:"));
    }
}

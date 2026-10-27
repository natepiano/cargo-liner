use std::fmt::Write as _;
use std::num::NonZeroUsize;

use super::ColorMode;
use super::CompilerStats;
use super::color;
use crate::reporting::constants::ANSI_BOLD_RED;
use crate::reporting::constants::CARGO_MEND_FIX;
use crate::reporting::constants::CARGO_MEND_FIX_ALL;
use crate::reporting::constants::CARGO_MEND_FIX_COMPILER;
use crate::reporting::constants::CARGO_MEND_FIX_PUB_USE;
use crate::reporting::constants::SUMMARY_LABEL;
use crate::reporting::diagnostics;
use crate::reporting::diagnostics::FixSummaryBucket;
use crate::reporting::diagnostics::Report;
use crate::reporting::diagnostics::Severity;

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

#[derive(Clone, Copy)]
enum FixableCategory {
    Unavailable,
    Available {
        count:   NonZeroUsize,
        command: &'static str,
    },
}

impl FixableCategory {
    const fn for_command(count: usize, command: &'static str) -> Self {
        match NonZeroUsize::new(count) {
            Some(count) => Self::Available { count, command },
            None => Self::Unavailable,
        }
    }

    const fn count(self) -> usize {
        match self {
            Self::Unavailable => 0,
            Self::Available { count, .. } => count.get(),
        }
    }

    fn append_row_entry(self, entries: &mut Vec<SummaryFixable>) {
        match self {
            Self::Unavailable => {},
            Self::Available { count, command } => {
                entries.push(SummaryFixable {
                    count: count.get(),
                    command,
                });
            },
        }
    }

    fn append_error_detail(self, details: &mut Vec<String>) {
        match self {
            Self::Unavailable => {},
            Self::Available { count, command } => {
                details.push(format!("{count} fixable with `{command}`"));
            },
        }
    }
}

#[derive(Clone, Copy)]
struct MendFixableCounts {
    standard: FixableCategory,
    pub_use:  FixableCategory,
}

impl MendFixableCounts {
    const fn total(self) -> usize { self.standard.count() + self.pub_use.count() }
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
    let fixables = mend_fixable_counts(report, Severity::Error);
    if fixables.total() == 0 {
        return Some(format!(
            "{} {n} {label} (not auto-fixable; fix manually)",
            color::paint("errors:", ANSI_BOLD_RED, color_mode)
        ));
    }

    let mut details = Vec::new();
    fixables.standard.append_error_detail(&mut details);
    fixables.pub_use.append_error_detail(&mut details);
    let manual = n.saturating_sub(fixables.total());
    if manual == 1 {
        details.push("1 requires a manual fix".to_string());
    } else if manual > 1 {
        details.push(format!("{manual} require manual fixes"));
    }
    Some(format!(
        "{} {n} {label} ({})",
        color::paint("errors:", ANSI_BOLD_RED, color_mode),
        details.join("; ")
    ))
}

fn mend_fixable_counts(report: &Report, severity: Severity) -> MendFixableCounts {
    let mut standard = 0;
    let mut pub_use = 0;
    for finding in report
        .findings
        .iter()
        .filter(|finding| diagnostics::effective_severity(finding) == severity)
    {
        match diagnostics::effective_fixability(finding).summary_bucket() {
            Some(FixSummaryBucket::Standard) => standard += 1,
            Some(FixSummaryBucket::PubUse) => pub_use += 1,
            None => {},
        }
    }
    MendFixableCounts {
        standard: FixableCategory::for_command(standard, CARGO_MEND_FIX),
        pub_use:  FixableCategory::for_command(pub_use, CARGO_MEND_FIX_PUB_USE),
    }
}

/// How many fix categories have at least one fixable item.
fn fixable_category_count(categories: [FixableCategory; 3]) -> usize {
    categories
        .into_iter()
        .map(|category| match category {
            FixableCategory::Unavailable => 0,
            FixableCategory::Available { .. } => 1,
        })
        .sum()
}

/// The roll-up printed under the findings, or `None` when there is nothing to
/// roll up.
///
/// Errors are reported by [`errors_block`] above this line and deliberately
/// never appear here, so a run whose only findings are errors leaves `rows`
/// empty. Saying "no issues found" there contradicts the block directly above
/// it, so that run gets no summary line at all.
pub(super) fn summary_line(
    report: &Report,
    compiler_stats: &CompilerStats,
    color_mode: ColorMode,
) -> Option<String> {
    let mut rows = Vec::new();
    let mend_fixables = mend_fixable_counts(report, Severity::Warning);
    let compiler_fixables =
        FixableCategory::for_command(compiler_stats.fixable, CARGO_MEND_FIX_COMPILER);
    let categories = fixable_category_count([
        mend_fixables.standard,
        mend_fixables.pub_use,
        compiler_fixables,
    ]);
    let total_fixable = mend_fixables.total() + compiler_fixables.count();

    if compiler_stats.warnings > 0 {
        let n = compiler_stats.warnings;
        let mut fixables = Vec::new();
        compiler_fixables.append_row_entry(&mut fixables);
        rows.push(SummaryRow {
            count: n,
            description: pluralize(n, "compiler warning", "compiler warnings").to_string(),
            fixables,
        });
    }
    if report.summary.warnings > 0 {
        let n = report.summary.warnings;
        let mut fixables = Vec::new();
        mend_fixables.standard.append_row_entry(&mut fixables);
        mend_fixables.pub_use.append_row_entry(&mut fixables);
        rows.push(SummaryRow {
            count: n,
            description: pluralize(n, "mend warning", "mend warnings").to_string(),
            fixables,
        });
    }

    if rows.is_empty() {
        if report.summary.errors > 0 {
            return None;
        }
        return Some(format!(
            "{} no issues found",
            color::dim(SUMMARY_LABEL, color_mode)
        ));
    }

    // When `categories > 1`, `summary_line` appends a `SummaryFixable` with `CARGO_MEND_FIX_ALL`
    // to the last warning row in `rows` so the single-command convergent option is
    // always one click away.
    if categories > 1
        && let Some(last) = rows.last_mut()
    {
        last.fixables.push(SummaryFixable {
            count:   total_fixable,
            command: CARGO_MEND_FIX_ALL,
        });
    }

    Some(render_summary_rows(&rows, color_mode))
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
    // `cont_indent` fills the `count_width` and `desc_width` columns so the dash
    // aligns with the inline `SummaryFixable` on the parent `SummaryRow`.
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
    use crate::compiler::DIAGNOSTIC_SEVERITY_WARNING_PREFIX;
    use crate::config::DiagnosticCode;
    use crate::reporting;
    use crate::reporting::ColorMode;
    use crate::reporting::CompilerStats;
    use crate::reporting::ItemVisibility;
    use crate::reporting::constants::CARGO_MEND_FIX;
    use crate::reporting::constants::CARGO_MEND_FIX_ALL;
    use crate::reporting::constants::CARGO_MEND_FIX_COMPILER;
    use crate::reporting::constants::CARGO_MEND_FIX_PUB_USE;
    use crate::reporting::constants::HINT_WARNING_FIXABLE_WITH_FIX;
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
                item_visibility: ItemVisibility::default(),
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
        assert!(!output.contains(DIAGNOSTIC_SEVERITY_WARNING_PREFIX));
    }

    #[test]
    fn render_human_report_shows_mend_summary_without_compiler_row() {
        let output = reporting::render_human_report(
            &mend_warning_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(output.contains(DIAGNOSTIC_SEVERITY_WARNING_PREFIX));
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

        // `output` gives the compiler row `CARGO_MEND_FIX_COMPILER` and the mend row
        // `CARGO_MEND_FIX` plus the `CARGO_MEND_FIX_ALL` continuation because two
        // categories are fixable.
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
        // `fix_all_row_dash` must equal `mend_row_dash`: the `CARGO_MEND_FIX_ALL` continuation
        // aligns under the inline fixable, with its dash in the mend row's dash column.
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
                item_visibility: ItemVisibility::default(),
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
                diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
                path:            "src/lib.rs".to_string(),
                line:            1,
                column:          1,
                highlight_len:   3,
                source_line:     "pub(in crate::internal) fn x() {}".to_string(),
                item:            Some("x".to_string()),
                message:         "forbidden".to_string(),
                suggestion:      None,
                fix_support:     FixSupport::None,
                related:         None,
                item_visibility: ItemVisibility::default(),
            }],
            ..Report::default()
        }
    }

    fn fixable_error_report() -> Report {
        let mut report = Report {
            root: ".".to_string(),
            findings: vec![Finding {
                severity:        Severity::Error,
                diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
                path:            "src/lib.rs".to_string(),
                line:            1,
                column:          1,
                highlight_len:   23,
                source_line:     "pub(in crate::internal) fn x() {}".to_string(),
                item:            Some("x".to_string()),
                message:         "restricted visibility does not match the required boundary"
                    .to_string(),
                suggestion:      Some("consider using: `pub(in crate::panel)`".to_string()),
                fix_support:     FixSupport::RestrictedAnnotation,
                related:         None,
                item_visibility: ItemVisibility::default(),
            }],
            ..Report::default()
        };
        report.refresh_summary();
        report
    }

    #[test]
    fn summary_never_emits_combined_fix_pub_use_string() {
        // `report` has both mend and pub-use fixables; `output` should render `CARGO_MEND_FIX`
        // and `CARGO_MEND_FIX_PUB_USE` on separate lines plus `CARGO_MEND_FIX_ALL`, never `cargo
        // mend --fix --fix-pub-use`.
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
            !output.contains(&format!(
                "{} {}",
                CARGO_MEND_FIX
                    .rsplit_once(' ')
                    .expect("fix command has a flag")
                    .1,
                CARGO_MEND_FIX_PUB_USE
                    .rsplit_once(' ')
                    .expect("pub-use command has a flag")
                    .1,
            )),
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
        // `report` models the bug: 213 mend warnings split 1-and-1 across
        // `CARGO_MEND_FIX` and `CARGO_MEND_FIX_PUB_USE`; `output` requires three lines.
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
        // `output` has a single fixable category, so it must omit `CARGO_MEND_FIX_ALL`.
        assert!(
            !output.contains(
                CARGO_MEND_FIX_ALL
                    .rsplit_once(' ')
                    .expect("fix-all command has a flag")
                    .1
            )
        );
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
        // `output` must never include errors in the "X fixable" warning summary count.
        assert!(!output.contains("mend errors -"));
    }

    /// A run whose only findings are errors must not report itself clean.
    /// `summary_line` rolls up warnings and `errors_block` reports errors, so
    /// an errors-only run leaves the summary's row set empty — which used to
    /// print "no issues found" directly beneath the block that had just
    /// reported 25 of them.
    #[test]
    fn errors_only_report_does_not_claim_no_issues_found() {
        let output = reporting::render_human_report(
            &errors_only_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(
            output.contains("3 mend errors"),
            "the errors block must still report the errors:\n{output}"
        );
        assert!(
            !output.contains("no issues found"),
            "a run with errors must not report itself clean:\n{output}"
        );
    }

    /// Severity is stored per finding, and only two sites write
    /// `Severity::Error` — the ones whose findings need a person to decide
    /// something. The cross-crate pass can later resolve one of those to an
    /// exact boundary and mark it fixable, and the stored severity does not
    /// follow. Reporting derives it instead, so one rule holds everywhere:
    /// anything mend can fix on its own is a warning, and an error is work only
    /// a person can do.
    #[test]
    fn a_fixable_finding_reports_as_a_warning_whatever_severity_it_carries() {
        let output = reporting::render_human_report(
            &fixable_error_report(),
            &compiler_stats(0, 0),
            ColorMode::Disabled,
        );

        assert!(
            output.contains(HINT_WARNING_FIXABLE_WITH_FIX),
            "a fixable finding must advertise its command as a warning:\n{output}"
        );
        assert!(
            output.contains("1 mend warning"),
            "a fixable finding belongs in the warning summary:\n{output}"
        );
        assert!(
            !output.contains("errors:"),
            "no error block when every finding is fixable:\n{output}"
        );
        assert!(!output.contains("not auto-fixable"));
    }

    #[test]
    fn warning_summary_counts_a_fixable_error_finding_among_the_fixable_warnings() {
        let mut report = mend_warning_report();
        report.findings.extend(fixable_error_report().findings);
        report.refresh_summary();

        let output =
            reporting::render_human_report(&report, &compiler_stats(0, 0), ColorMode::Disabled);
        let warning_row = output
            .lines()
            .find(|line| line.contains("mend warning"))
            .expect("missing mend warning summary row");

        assert!(
            warning_row.contains("2 mend warnings"),
            "a fixable finding recorded as an error still reports as a warning:\n{output}"
        );
        assert!(
            warning_row.contains(&format!("2 fixable with `{CARGO_MEND_FIX}`")),
            "both fixable findings belong in the warning row's fixable count:\n{output}"
        );
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
    #[test]
    fn all_category_combinations_preserve_commands_and_aggregate_counts() {
        for bits in 0_u8..8 {
            let mut report = Report::default();
            if bits & 1 != 0 {
                report.findings.extend(mend_warning_report().findings);
            }
            if bits & 2 != 0 {
                report.findings.extend(pub_use_warning_report().findings);
            }
            report.refresh_summary();
            let compiler = usize::from(bits & 4 != 0);
            let output = super::summary_line(
                &report,
                &compiler_stats(compiler, compiler),
                ColorMode::Disabled,
            )
            .expect("summary is present for warnings or clean output");
            for (mask, command) in [
                (1, CARGO_MEND_FIX),
                (2, CARGO_MEND_FIX_PUB_USE),
                (4, CARGO_MEND_FIX_COMPILER),
            ] {
                assert_eq!(
                    output.contains(&format!("`{command}`")),
                    bits & mask != 0,
                    "category mask {bits}: {output}"
                );
            }
            let count = bits.count_ones();
            assert_eq!(output.contains(CARGO_MEND_FIX_ALL), count > 1, "{output}");
            if count > 1 {
                assert!(
                    output.contains(&format!("{count} fixable with `{CARGO_MEND_FIX_ALL}`")),
                    "{output}"
                );
            }
        }
    }
}

use super::ColorMode;
use super::CompilerStats;
use super::diagnostic;
use super::summary;
use crate::reporting::diagnostics::Report;

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
            diagnostic::render_finding(&mut output, finding, color_mode);
        }
    }

    if let Some(errors_block) = summary::errors_block(report, color_mode) {
        output.push_str(&errors_block);
        output.push('\n');
    }
    output.push_str(&summary::summary_line(report, compiler_stats, color_mode));
    output.push('\n');
    output
}

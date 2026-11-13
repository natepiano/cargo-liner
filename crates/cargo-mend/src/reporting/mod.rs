mod cargo_json;
mod constants;
mod diagnostics;
mod outcome;
mod render;

pub(crate) use cargo_json::render_report;
pub(crate) use constants::CARGO_TERM_COLOR_ALWAYS;
pub(crate) use constants::CARGO_TERM_COLOR_ENV;
pub(crate) use constants::CARGO_TERM_COLOR_NEVER;
pub(crate) use constants::CLICOLOR_DISABLED_VALUE;
pub(crate) use constants::CLICOLOR_ENV;
pub(crate) use constants::CLICOLOR_FORCE_ENV;
pub(crate) use constants::DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH;
pub(crate) use constants::EXIT_CODE_ERROR;
pub(crate) use constants::EXIT_CODE_WARNING;
pub(crate) use constants::PUB_USE_OUTSIDE_SUBTREE_HELP;
pub(crate) use diagnostics::AllFeaturesCoverage;
pub(crate) use diagnostics::AncestorReexport;
pub(crate) use diagnostics::AncestorReexportInsertion;
pub(crate) use diagnostics::BuildOutcome;
pub(crate) use diagnostics::CompilerWarningFacts;
pub(crate) use diagnostics::ExactBoundarySpelling;
pub(crate) use diagnostics::Finding;
pub(crate) use diagnostics::FixSupport;
pub(crate) use diagnostics::ItemVisibility;
pub(crate) use diagnostics::ModuleForm;
pub(crate) use diagnostics::ModuleMountFact;
pub(crate) use diagnostics::NarrowerScope;
pub(crate) use diagnostics::PubUseFixFact;
pub(crate) use diagnostics::ReexportVisibility;
pub(crate) use diagnostics::Report;
pub(crate) use diagnostics::ReportFacts;
pub(crate) use diagnostics::ReportSummary;
pub(crate) use diagnostics::Severity;
pub(crate) use diagnostics::SubtreeReexportFixFact;
pub(crate) use diagnostics::WarningPresence;
pub(crate) use diagnostics::WrittenVisibility;
pub(crate) use outcome::AnalysisFailure;
pub(crate) use outcome::AppliedFixCounts;
pub(crate) use outcome::AppliedFixes;
pub(crate) use outcome::CompilerFailureCause;
pub(crate) use outcome::ExecutionNotice;
pub(crate) use outcome::ExecutionOutcome;
pub(crate) use outcome::FixKind;
pub(crate) use outcome::FixNotice;
pub(crate) use outcome::FixValidationFailure;
pub(crate) use outcome::MendFailure;
pub(crate) use outcome::NoticeKind;
pub(crate) use outcome::PassEdits;
pub(crate) use outcome::PubUseNotice;
pub(crate) use outcome::RollbackStatus;
pub(crate) use outcome::SubtreeReexportEdits;
pub(crate) use render::ColorMode;
pub(crate) use render::CompilerStats;
pub(crate) use render::OutputFormat;
pub(crate) use render::render_human_report;
pub(crate) use render::render_timing;

/// Preserve compiler input order and the runner's suggestion-sensitive duplicate policy.
pub(crate) fn sort_and_dedup_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        (
            a.input_order(),
            &a.path,
            a.line,
            a.column,
            &a.diagnostic_code,
            &a.item,
            &a.message,
            &a.suggestion,
        )
            .cmp(&(
                b.input_order(),
                &b.path,
                b.line,
                b.column,
                &b.diagnostic_code,
                &b.item,
                &b.message,
                &b.suggestion,
            ))
    });
    findings.dedup_by(|a, b| {
        a.input_order() == b.input_order()
            && a.diagnostic_code == b.diagnostic_code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
            && a.suggestion == b.suggestion
    });
}

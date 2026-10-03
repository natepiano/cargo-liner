use super::MendRunner;
use super::RunPlan;
use crate::compiler;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::OperationIntent;
use crate::fixes::imports;
use crate::fixes::imports::ImportScan;
use crate::fixes::imports_at_top;
use crate::fixes::imports_at_top::ImportsAtTopScan;
use crate::fixes::inline_path_qualified_type;
use crate::fixes::inline_path_qualified_type::InlinePathScan;
use crate::fixes::prefer_module_import;
use crate::fixes::prefer_module_import::PreferModuleImportScan;
use crate::reporting::ExecutionOutcome;
use crate::reporting::MendFailure;
use crate::reporting::PassEdits;
use crate::rust_syntax::ParsedSources;

/// A checked selection, with the syntax scans that added findings to its
/// report and the sources they read, kept so the fix scans that follow reuse
/// both instead of reading the tree again.
pub(super) struct CheckedSelection {
    pub(super) result:  SelectionResult,
    pub(super) scans:   SyntaxScans,
    pub(super) sources: ParsedSources,
}

/// The syntax scans whose diagnostics are enabled; a fix kind is enabled only
/// with one of its diagnostics, so each fix scan finds its scan here.
pub(super) struct SyntaxScans {
    pub(super) imports:              Option<ImportScan>,
    pub(super) prefer_module_import: Option<PreferModuleImportScan>,
    pub(super) inline_path:          Option<InlinePathScan>,
    pub(super) imports_at_top:       Option<ImportsAtTopScan>,
}

impl MendRunner<'_> {
    pub(super) fn execute(&mut self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        match planned.operation_mode.intent {
            OperationIntent::ReadOnly => Ok(ExecutionOutcome {
                compiler_warning_facts: planned.report.facts.compiler_warning_facts,
                report: planned.report,
                notice: None,
                check_duration,
                compiler_warnings,
                compiler_fixable,
                applied_pub_use: 0,
                applied_subtree_reexport: false,
                pass_edits: PassEdits::Unchanged,
            }),
            OperationIntent::DryRun => {
                // Count the validated set, not the scans: a dry run must
                // preview the edits an apply would write, and combining is
                // where conflicting groups are dropped and duplicates collapse.
                let fix_scans = planned.fix_scans();
                let previewed = Self::combined_fixes(fix_scans)?;
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&planned.report),
                    fix_scans,
                    previewed.counts(),
                );
                Ok(ExecutionOutcome {
                    compiler_warning_facts: planned.report.facts.compiler_warning_facts,
                    report: planned.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use: 0,
                    applied_subtree_reexport: false,
                    pass_edits: PassEdits::Unchanged,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    pub(super) fn build_selection(
        &self,
        output_mode: BuildOutputMode,
    ) -> Result<CheckedSelection, MendFailure> {
        let mut result = compiler::run_selection(
            self.selection,
            self.cargo_plan,
            self.loaded_config,
            self.clippy_status,
            output_mode,
            self.color_mode,
        )?;
        // Created after the check, which `cargo fix` may have rewritten files
        // during.
        let sources = ParsedSources::default();
        let diagnostics_config = &self.loaded_config.diagnostics_config;
        let enabled = |codes: &[DiagnosticCode]| {
            codes
                .iter()
                .any(|&code| diagnostics_config.is_enabled(code) == DiagnosticStatus::Enabled)
        };
        let scans = SyntaxScans {
            imports:              enabled(&[
                DiagnosticCode::ShortenLocalCrateImport,
                DiagnosticCode::ReplaceDeepSuperImport,
            ])
            .then(|| imports::scan_selection(self.selection, &sources))
            .transpose()
            .map_err(MendFailure::Unexpected)?,
            prefer_module_import: enabled(&[DiagnosticCode::PreferModuleImport])
                .then(|| prefer_module_import::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
            inline_path:          enabled(&[DiagnosticCode::InlinePathQualifiedType])
                .then(|| inline_path_qualified_type::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
            imports_at_top:       enabled(&[DiagnosticCode::ImportsAtTop])
                .then(|| imports_at_top::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
        };
        let report = &mut result.report;
        let scan_findings = [
            scans.imports.as_ref().map(|scan| &scan.findings),
            scans
                .prefer_module_import
                .as_ref()
                .map(|scan| &scan.findings),
            scans.inline_path.as_ref().map(|scan| &scan.findings),
            scans.imports_at_top.as_ref().map(|scan| &scan.findings),
        ];
        report
            .findings
            .extend(scan_findings.into_iter().flatten().flatten().cloned());
        report.findings.sort_by(|a, b| {
            (
                a.severity,
                &a.path,
                a.line,
                a.column,
                &a.diagnostic_code,
                &a.item,
                &a.message,
                &a.suggestion,
            )
                .cmp(&(
                    b.severity,
                    &b.path,
                    b.line,
                    b.column,
                    &b.diagnostic_code,
                    &b.item,
                    &b.message,
                    &b.suggestion,
                ))
        });
        report.findings.dedup_by(|a, b| {
            a.severity == b.severity
                && a.diagnostic_code == b.diagnostic_code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
                && a.suggestion == b.suggestion
        });
        // Filter out disabled diagnostics
        report.findings.retain(|f| {
            self.loaded_config
                .diagnostics_config
                .is_enabled(f.diagnostic_code)
                == DiagnosticStatus::Enabled
        });
        report.refresh_summary();
        Ok(CheckedSelection {
            result,
            scans,
            sources,
        })
    }
}

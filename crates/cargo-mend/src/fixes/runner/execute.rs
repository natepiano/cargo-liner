use super::MendRunner;
use super::RunPlan;
use crate::compiler;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::OperationIntent;
use crate::fixes::imports;
use crate::fixes::imports_at_top;
use crate::fixes::inline_path_qualified_type;
use crate::fixes::prefer_module_import;
use crate::reporting::ExecutionOutcome;
use crate::reporting::MendFailure;

impl MendRunner<'_> {
    pub(super) fn execute(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
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
            }),
            OperationIntent::DryRun => {
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&planned.report),
                    planned.fix_scans(),
                );
                Ok(ExecutionOutcome {
                    compiler_warning_facts: planned.report.facts.compiler_warning_facts,
                    report: planned.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use: 0,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    pub(super) fn build_selection(
        &self,
        output_mode: BuildOutputMode,
    ) -> Result<SelectionResult, MendFailure> {
        let mut result = compiler::run_selection(
            self.selection,
            self.cargo_plan,
            self.loaded_config,
            output_mode,
            self.color_mode,
        )?;
        let report = &mut result.report;
        let diagnostics_config = &self.loaded_config.diagnostics_config;
        if diagnostics_config.is_enabled(DiagnosticCode::ShortenLocalCrateImport)
            == DiagnosticStatus::Enabled
            || diagnostics_config.is_enabled(DiagnosticCode::ReplaceDeepSuperImport)
                == DiagnosticStatus::Enabled
        {
            let import_scan =
                imports::scan_selection(self.selection).map_err(MendFailure::Unexpected)?;
            report.findings.extend(import_scan.findings);
        }
        if diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport)
            == DiagnosticStatus::Enabled
        {
            let prefer_module_import_scan = prefer_module_import::scan_selection(self.selection)
                .map_err(MendFailure::Unexpected)?;
            report.findings.extend(prefer_module_import_scan.findings);
        }
        if diagnostics_config.is_enabled(DiagnosticCode::InlinePathQualifiedType)
            == DiagnosticStatus::Enabled
        {
            let inline_path_scan = inline_path_qualified_type::scan_selection(self.selection)
                .map_err(MendFailure::Unexpected)?;
            report.findings.extend(inline_path_scan.findings);
        }
        if diagnostics_config.is_enabled(DiagnosticCode::ImportsAtTop) == DiagnosticStatus::Enabled
        {
            let imports_at_top_scan =
                imports_at_top::scan_selection(self.selection).map_err(MendFailure::Unexpected)?;
            report.findings.extend(imports_at_top_scan.findings);
        }
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
        Ok(result)
    }
}

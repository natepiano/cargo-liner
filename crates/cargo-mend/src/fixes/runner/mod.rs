use std::time::Duration;

use anyhow::Result;

use super::field_visibility::FieldVisibilityFixScan;
use super::imports;
use super::imports::ImportScan;
use super::imports_at_top;
use super::imports_at_top::ImportsAtTopScan;
use super::inline_path_qualified_type;
use super::inline_path_qualified_type::InlinePathScan;
use super::narrow_pub_crate::NarrowPubCrateScan;
use super::prefer_module_import;
use super::prefer_module_import::PreferModuleImportScan;
use super::pub_use_fixes::PubUseFixScan;
use super::unused_pub::UnusedPubScan;
use crate::compiler;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::LoadedConfig;
use crate::config::OperationIntent;
use crate::config::OperationMode;
use crate::reporting::ColorMode;
use crate::reporting::ExecutionOutcome;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;
use crate::reporting::Report;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

mod apply;
mod combine;
mod notices;
mod plan;

pub(crate) struct MendRunner<'a> {
    pub(super) selection:     &'a Selection,
    pub(super) cargo_plan:    &'a CargoCheckPlan,
    pub(super) loaded_config: &'a LoadedConfig,
    pub(super) color_mode:    ColorMode,
    pub(super) output_format: OutputFormat,
}

struct RunPlan {
    operation_mode:            OperationMode,
    report:                    Report,
    import_scan:               Option<ImportScan>,
    prefer_module_import_scan: Option<PreferModuleImportScan>,
    inline_path_scan:          Option<InlinePathScan>,
    unused_pub_scan:           Option<UnusedPubScan>,
    narrow_pub_crate_scan:     Option<NarrowPubCrateScan>,
    field_visibility_fix_scan: Option<FieldVisibilityFixScan>,
    imports_at_top_scan:       Option<ImportsAtTopScan>,
    pub_use_scan:              Option<PubUseFixScan>,
    check_duration:            Duration,
    compiler_warnings:         usize,
    compiler_fixable:          usize,
}

#[derive(Clone, Copy)]
struct FixScans<'a> {
    imports:          Option<&'a ImportScan>,
    module_imports:   Option<&'a PreferModuleImportScan>,
    inline_types:     Option<&'a InlinePathScan>,
    unused_pub:       Option<&'a UnusedPubScan>,
    narrowed_pub:     Option<&'a NarrowPubCrateScan>,
    field_visibility: Option<&'a FieldVisibilityFixScan>,
    imports_at_top:   Option<&'a ImportsAtTopScan>,
    pub_use:          Option<&'a PubUseFixScan>,
}

impl RunPlan {
    const fn fix_scans(&self) -> FixScans<'_> {
        FixScans {
            imports:          self.import_scan.as_ref(),
            module_imports:   self.prefer_module_import_scan.as_ref(),
            inline_types:     self.inline_path_scan.as_ref(),
            unused_pub:       self.unused_pub_scan.as_ref(),
            narrowed_pub:     self.narrow_pub_crate_scan.as_ref(),
            field_visibility: self.field_visibility_fix_scan.as_ref(),
            imports_at_top:   self.imports_at_top_scan.as_ref(),
            pub_use:          self.pub_use_scan.as_ref(),
        }
    }
}

impl<'a> MendRunner<'a> {
    pub(crate) const fn new(
        selection: &'a Selection,
        cargo_plan: &'a CargoCheckPlan,
        loaded_config: &'a LoadedConfig,
        color_mode: ColorMode,
        output_format: OutputFormat,
    ) -> Self {
        Self {
            selection,
            cargo_plan,
            loaded_config,
            color_mode,
            output_format,
        }
    }

    pub(crate) fn run(
        &self,
        operation_mode: OperationMode,
    ) -> Result<ExecutionOutcome, MendFailure> {
        let planned = self.plan(operation_mode)?;
        self.execute(planned)
    }

    fn execute(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
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

    fn build_selection(
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

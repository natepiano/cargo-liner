use std::time::Duration;

use anyhow::Result;

use super::compiler;
use super::compiler::BuildOutputMode;
use super::compiler::SelectionResult;
use super::config::DiagnosticCode;
use super::config::LoadedConfig;
use super::diagnostics::Report;
use super::imports;
use super::imports::ImportScan;
use super::imports::ValidatedFixSet;
use super::inline_path_qualified_type;
use super::inline_path_qualified_type::InlinePathScan;
use super::narrow_pub_crate;
use super::narrow_pub_crate::NarrowPubCrateScan;
use super::outcome::CompilerFailureCause;
use super::outcome::ExecutionNotice;
use super::outcome::ExecutionOutcome;
use super::outcome::FixNotice;
use super::outcome::FixValidationFailure;
use super::outcome::MendFailure;
use super::outcome::NoticeKind;
use super::outcome::PubUseNotice;
use super::outcome::RollbackStatus;
use super::prefer_module_import;
use super::prefer_module_import::PreferModuleImportScan;
use super::pub_use_fixes;
use super::pub_use_fixes::PubUseFixScan;
use super::render;
use super::run_mode::FixKind;
use super::run_mode::OperationIntent;
use super::run_mode::OperationMode;
use super::selection::CargoCheckPlan;
use super::selection::Selection;

pub(crate) struct MendRunner<'a> {
    selection:  &'a Selection,
    cargo_plan: &'a CargoCheckPlan,
    config:     &'a LoadedConfig,
    color:      render::ColorMode,
    output:     render::OutputFormat,
}

struct RunPlan {
    mode:                      OperationMode,
    report:                    Report,
    import_scan:               Option<ImportScan>,
    prefer_module_import_scan: Option<PreferModuleImportScan>,
    inline_path_scan:          Option<InlinePathScan>,
    narrow_pub_crate_scan:     Option<NarrowPubCrateScan>,
    pub_use_scan:              Option<PubUseFixScan>,
    check_duration:            Duration,
    compiler_warnings:         usize,
    compiler_fixable:          usize,
}

#[derive(Clone, Copy)]
struct FixScans<'a> {
    imports:        Option<&'a ImportScan>,
    module_imports: Option<&'a PreferModuleImportScan>,
    inline_types:   Option<&'a InlinePathScan>,
    narrowed_pub:   Option<&'a NarrowPubCrateScan>,
    pub_use:        Option<&'a PubUseFixScan>,
}

impl RunPlan {
    const fn fix_scans(&self) -> FixScans<'_> {
        FixScans {
            imports:        self.import_scan.as_ref(),
            module_imports: self.prefer_module_import_scan.as_ref(),
            inline_types:   self.inline_path_scan.as_ref(),
            narrowed_pub:   self.narrow_pub_crate_scan.as_ref(),
            pub_use:        self.pub_use_scan.as_ref(),
        }
    }
}

impl<'a> MendRunner<'a> {
    pub(crate) const fn new(
        selection: &'a Selection,
        cargo_plan: &'a CargoCheckPlan,
        config: &'a LoadedConfig,
        color: render::ColorMode,
        output: render::OutputFormat,
    ) -> Self {
        Self {
            selection,
            cargo_plan,
            config,
            color,
            output,
        }
    }

    pub(crate) fn run(&self, mode: OperationMode) -> Result<ExecutionOutcome, MendFailure> {
        let planned = self.plan(mode)?;
        self.execute(planned)
    }

    fn plan(&self, mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let output_mode = if self.output == render::OutputFormat::Json {
            BuildOutputMode::Json
        } else if mode.fixes.contains(FixKind::FixPubUse) {
            BuildOutputMode::SuppressUnusedImportWarnings
        } else {
            BuildOutputMode::Full
        };
        let selection_result = self.build_selection(output_mode)?;
        let report = selection_result.report;
        let check_duration = selection_result.check_duration;
        let compiler_warnings = selection_result.compiler_warnings;
        let compiler_fixable = selection_result.compiler_fixable;
        let diagnostics = &self.config.diagnostics;
        let import_scan = (mode.fixes.contains(FixKind::ShortenImport)
            && (diagnostics.is_enabled(DiagnosticCode::ShortenLocalCrateImport)
                || diagnostics.is_enabled(DiagnosticCode::ReplaceDeepSuperImport)))
        .then(|| imports::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let prefer_module_import_scan = (mode.fixes.contains(FixKind::PreferModuleImport)
            && diagnostics.is_enabled(DiagnosticCode::PreferModuleImport))
        .then(|| prefer_module_import::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let inline_path_scan = (mode.fixes.contains(FixKind::InlinePathQualifiedType)
            && diagnostics.is_enabled(DiagnosticCode::InlinePathQualifiedType))
        .then(|| inline_path_qualified_type::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let narrow_pub_crate_scan = (mode.fixes.contains(FixKind::NarrowToPubCrate)
            && diagnostics.is_enabled(DiagnosticCode::NarrowToPubCrate))
        .then(|| narrow_pub_crate::scan_from_report(&report))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let pub_use_scan = mode
            .fixes
            .contains(FixKind::FixPubUse)
            .then(|| pub_use_fixes::scan_selection(self.selection, &report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;

        Ok(RunPlan {
            mode,
            report,
            import_scan,
            prefer_module_import_scan,
            inline_path_scan,
            narrow_pub_crate_scan,
            pub_use_scan,
            check_duration,
            compiler_warnings,
            compiler_fixable,
        })
    }

    fn execute(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        match planned.mode.intent {
            OperationIntent::ReadOnly => Ok(ExecutionOutcome {
                report: planned.report,
                notice: None,
                check_duration,
                compiler_warnings,
                compiler_fixable,
            }),
            OperationIntent::DryRun => {
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&planned.report),
                    planned.fix_scans(),
                );
                Ok(ExecutionOutcome {
                    report: planned.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    fn apply(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let plan_check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        let fix_scans = planned.fix_scans();
        let fixes = Self::combined_fixes(fix_scans)?;
        if fixes.is_empty() {
            let notice =
                Self::build_fix_notice(planned.mode.intent, Some(&planned.report), fix_scans);
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
                check_duration: plan_check_duration,
                compiler_warnings,
                compiler_fixable,
            });
        }

        let snapshots = imports::snapshot_files(&fixes).map_err(MendFailure::Unexpected)?;
        imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        match self.build_selection(BuildOutputMode::Quiet) {
            Ok(validation) => {
                let check_duration = plan_check_duration + validation.check_duration;
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&validation.report),
                    fix_scans,
                );
                Ok(ExecutionOutcome {
                    report: validation.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                })
            },
            Err(err) => {
                let rollback = match imports::restore_files(&snapshots) {
                    Ok(()) => RollbackStatus::Restored,
                    Err(_) => RollbackStatus::RestoreFailed,
                };
                let cause = match err {
                    MendFailure::Analysis(a) => a.cause,
                    MendFailure::Unexpected(e) => CompilerFailureCause::Unexpected(e),
                    MendFailure::FixValidation(f) => f.cause,
                };
                Err(MendFailure::FixValidation(FixValidationFailure {
                    rollback,
                    cause,
                }))
            },
        }
    }

    fn combined_fixes(fix_scans: FixScans<'_>) -> Result<ValidatedFixSet, MendFailure> {
        // Collect `prefer_module_import` fix ranges for deconfliction with `ShortenImport`
        let prefer_ranges: Vec<(&std::path::Path, usize, usize)> = fix_scans
            .module_imports
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .map(|fix| (fix.path.as_path(), fix.start, fix.end))
            .collect();

        let mut fixes = Vec::new();

        // Add `ShortenImport` fixes, filtering out any that overlap with `PreferModuleImport`
        if let Some(scan) = fix_scans.imports {
            for fix in scan.fixes.iter() {
                let overlaps = prefer_ranges.iter().any(|(path, start, end)| {
                    fix.path.as_path() == *path && fix.start < *end && *start < fix.end
                });
                if !overlaps {
                    fixes.push(fix.clone());
                }
            }
        }
        if let Some(scan) = fix_scans.module_imports {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.inline_types {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.narrowed_pub {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.pub_use {
            fixes.extend(scan.fixes.iter().cloned());
        }
        imports::ValidatedFixSet::from_vec(fixes).map_err(MendFailure::Unexpected)
    }

    fn build_fix_notice(
        intent: OperationIntent,
        report: Option<&Report>,
        fix_scans: FixScans<'_>,
    ) -> Option<ExecutionNotice> {
        let mut notices = Vec::new();
        let import_fix_count = fix_scans.imports.map_or(0, |scan| scan.findings.len())
            + fix_scans
                .module_imports
                .map_or(0, |scan| scan.findings.len())
            + fix_scans.inline_types.map_or(0, |scan| scan.findings.len())
            + fix_scans.narrowed_pub.map_or(0, |scan| scan.fixes.len());
        if fix_scans.imports.is_some()
            || fix_scans.module_imports.is_some()
            || fix_scans.inline_types.is_some()
            || fix_scans.narrowed_pub.is_some()
        {
            notices.push(NoticeKind::ImportFixes(FixNotice::from_intent(
                intent,
                import_fix_count,
            )));
        }

        if let Some(scan) = fix_scans.pub_use {
            notices.push(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
                intent,
                scan.applied,
                scan.skipped,
            )));
        }

        if matches!(intent, OperationIntent::Apply)
            && fix_scans.pub_use.is_some_and(|scan| scan.applied > 0)
            && report
                .is_some_and(|report| report.facts.compiler_warnings.saw_unused_import_warnings())
        {
            notices.push(NoticeKind::ImportCleanupSuggested);
        }

        match notices.len() {
            0 => None,
            1 => notices.into_iter().next().map(ExecutionNotice::from),
            _ => Some(ExecutionNotice::from(notices)),
        }
    }

    fn build_selection(
        &self,
        output_mode: BuildOutputMode,
    ) -> Result<SelectionResult, MendFailure> {
        let mut result = compiler::run_selection(
            self.selection,
            self.cargo_plan,
            self.config,
            output_mode,
            self.color,
        )?;
        let report = &mut result.report;
        let diagnostics = &self.config.diagnostics;
        if diagnostics.is_enabled(DiagnosticCode::ShortenLocalCrateImport)
            || diagnostics.is_enabled(DiagnosticCode::ReplaceDeepSuperImport)
        {
            let import_scan =
                imports::scan_selection(self.selection).map_err(MendFailure::Unexpected)?;
            report.findings.extend(import_scan.findings);
        }
        if diagnostics.is_enabled(DiagnosticCode::PreferModuleImport) {
            let prefer_module_import_scan = prefer_module_import::scan_selection(self.selection)
                .map_err(MendFailure::Unexpected)?;
            report.findings.extend(prefer_module_import_scan.findings);
        }
        if diagnostics.is_enabled(DiagnosticCode::InlinePathQualifiedType) {
            let inline_path_scan = inline_path_qualified_type::scan_selection(self.selection)
                .map_err(MendFailure::Unexpected)?;
            report.findings.extend(inline_path_scan.findings);
        }
        report.findings.sort_by(|a, b| {
            (
                a.severity,
                &a.path,
                a.line,
                a.column,
                &a.code,
                &a.item,
                &a.message,
                &a.suggestion,
            )
                .cmp(&(
                    b.severity,
                    &b.path,
                    b.line,
                    b.column,
                    &b.code,
                    &b.item,
                    &b.message,
                    &b.suggestion,
                ))
        });
        report.findings.dedup_by(|a, b| {
            a.severity == b.severity
                && a.code == b.code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
                && a.suggestion == b.suggestion
        });
        // Filter out disabled diagnostics
        report
            .findings
            .retain(|f| self.config.diagnostics.is_enabled(f.code));
        report.refresh_summary();
        Ok(result)
    }
}

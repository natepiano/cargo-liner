use anyhow::Result;

use super::compiler;
use super::compiler::BuildOutputMode;
use super::config::DiagnosticCode;
use super::config::LoadedConfig;
use super::diagnostics::Report;
use super::imports;
use super::imports::ImportScan;
use super::imports::ValidatedFixSet;
use super::inline_path_qualified_type;
use super::inline_path_qualified_type::InlinePathScan;
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
use super::run_mode::FixKind;
use super::run_mode::OperationIntent;
use super::run_mode::OperationMode;
use super::selection::Selection;

pub(crate) struct MendRunner<'a> {
    selection: &'a Selection,
    config:    &'a LoadedConfig,
}

struct RunPlan {
    mode:                      OperationMode,
    report:                    Report,
    import_scan:               Option<ImportScan>,
    prefer_module_import_scan: Option<PreferModuleImportScan>,
    inline_path_scan:          Option<InlinePathScan>,
    pub_use_scan:              Option<PubUseFixScan>,
}

impl<'a> MendRunner<'a> {
    pub const fn new(selection: &'a Selection, config: &'a LoadedConfig) -> Self {
        Self { selection, config }
    }

    pub fn run(&self, mode: OperationMode) -> Result<ExecutionOutcome, MendFailure> {
        let planned = self.plan(mode)?;
        self.execute(planned)
    }

    fn plan(&self, mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let output_mode = if mode.fixes.contains(FixKind::FixPubUse) {
            BuildOutputMode::SuppressUnusedImportWarnings
        } else {
            BuildOutputMode::Full
        };
        let report = self.build_report(output_mode)?;
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
            pub_use_scan,
        })
    }

    fn execute(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        match planned.mode.intent {
            OperationIntent::ReadOnly => Ok(ExecutionOutcome {
                report: planned.report,
                notice: None,
            }),
            OperationIntent::DryRun => {
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&planned.report),
                    planned.import_scan.as_ref(),
                    planned.prefer_module_import_scan.as_ref(),
                    planned.inline_path_scan.as_ref(),
                    planned.pub_use_scan.as_ref(),
                );
                Ok(ExecutionOutcome {
                    report: planned.report,
                    notice,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    fn apply(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let fixes = Self::combined_fixes(
            planned.import_scan.as_ref(),
            planned.prefer_module_import_scan.as_ref(),
            planned.inline_path_scan.as_ref(),
            planned.pub_use_scan.as_ref(),
        )?;
        if fixes.is_empty() {
            let notice = Self::build_fix_notice(
                planned.mode.intent,
                Some(&planned.report),
                planned.import_scan.as_ref(),
                planned.prefer_module_import_scan.as_ref(),
                planned.inline_path_scan.as_ref(),
                planned.pub_use_scan.as_ref(),
            );
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
            });
        }

        let snapshots = imports::snapshot_files(&fixes).map_err(MendFailure::Unexpected)?;
        imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        match self.build_report(BuildOutputMode::Full) {
            Ok(report) => {
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&report),
                    planned.import_scan.as_ref(),
                    planned.prefer_module_import_scan.as_ref(),
                    planned.inline_path_scan.as_ref(),
                    planned.pub_use_scan.as_ref(),
                );
                Ok(ExecutionOutcome { report, notice })
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

    fn combined_fixes(
        import_scan: Option<&ImportScan>,
        prefer_module_import_scan: Option<&PreferModuleImportScan>,
        inline_path_scan: Option<&InlinePathScan>,
        pub_use_scan: Option<&PubUseFixScan>,
    ) -> Result<ValidatedFixSet, MendFailure> {
        // Collect `prefer_module_import` fix ranges for deconfliction with `ShortenImport`
        let prefer_ranges: Vec<(&std::path::Path, usize, usize)> = prefer_module_import_scan
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .map(|fix| (fix.path.as_path(), fix.start, fix.end))
            .collect();

        let mut fixes = Vec::new();

        // Add `ShortenImport` fixes, filtering out any that overlap with `PreferModuleImport`
        if let Some(scan) = import_scan {
            for fix in scan.fixes.iter() {
                let overlaps = prefer_ranges.iter().any(|(path, start, end)| {
                    fix.path.as_path() == *path && fix.start < *end && *start < fix.end
                });
                if !overlaps {
                    fixes.push(fix.clone());
                }
            }
        }
        if let Some(scan) = prefer_module_import_scan {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = inline_path_scan {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = pub_use_scan {
            fixes.extend(scan.fixes.iter().cloned());
        }
        imports::ValidatedFixSet::from_vec(fixes).map_err(MendFailure::Unexpected)
    }

    fn build_fix_notice(
        intent: OperationIntent,
        report: Option<&Report>,
        import_scan: Option<&ImportScan>,
        prefer_module_import_scan: Option<&PreferModuleImportScan>,
        inline_path_scan: Option<&InlinePathScan>,
        pub_use_scan: Option<&PubUseFixScan>,
    ) -> Option<ExecutionNotice> {
        let mut notices = Vec::new();
        let import_fix_count = import_scan.map_or(0, |s| s.findings.len())
            + prefer_module_import_scan.map_or(0, |s| s.findings.len())
            + inline_path_scan.map_or(0, |s| s.findings.len());
        if import_scan.is_some()
            || prefer_module_import_scan.is_some()
            || inline_path_scan.is_some()
        {
            notices.push(NoticeKind::ImportFixes(FixNotice::from_intent(
                intent,
                import_fix_count,
            )));
        }

        if let Some(scan) = pub_use_scan {
            notices.push(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
                intent,
                scan.applied_count,
                scan.skipped_count,
            )));
        }

        if matches!(intent, OperationIntent::Apply)
            && pub_use_scan.is_some_and(|scan| scan.applied_count > 0)
            && report
                .is_some_and(|report| report.facts.compiler_warnings.saw_unused_import_warnings())
        {
            notices.push(NoticeKind::ImportCleanupSuggested);
        }

        match notices.len() {
            0 => None,
            1 => notices.into_iter().next().map(ExecutionNotice::from_kind),
            _ => Some(ExecutionNotice::from_kinds(notices)),
        }
    }

    fn build_report(&self, output_mode: BuildOutputMode) -> Result<Report, MendFailure> {
        let mut report = compiler::run_selection(self.selection, self.config, output_mode)?;
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
        Ok(report)
    }
}

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use super::field_visibility;
use super::field_visibility::FieldVisibilityFixScan;
use super::imports;
use super::imports::ImportScan;
use super::imports::ValidatedFixSet;
use super::imports_at_top;
use super::imports_at_top::ImportsAtTopScan;
use super::inline_path_qualified_type;
use super::inline_path_qualified_type::InlinePathScan;
use super::narrow_pub_crate;
use super::narrow_pub_crate::NarrowPubCrateScan;
use super::prefer_module_import;
use super::prefer_module_import::PreferModuleImportScan;
use super::pub_use_fixes;
use super::pub_use_fixes::PubUseFixScan;
use super::unused_pub;
use super::unused_pub::UnusedPubScan;
use crate::compiler;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::FixKind;
use crate::config::LoadedConfig;
use crate::config::OperationIntent;
use crate::config::OperationMode;
use crate::reporting::ColorMode;
use crate::reporting::CompilerFailureCause;
use crate::reporting::ExecutionNotice;
use crate::reporting::ExecutionOutcome;
use crate::reporting::FixNotice;
use crate::reporting::FixValidationFailure;
use crate::reporting::MendFailure;
use crate::reporting::NoticeKind;
use crate::reporting::OutputFormat;
use crate::reporting::PubUseNotice;
use crate::reporting::Report;
use crate::reporting::RollbackStatus;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

pub(crate) struct MendRunner<'a> {
    selection:     &'a Selection,
    cargo_plan:    &'a CargoCheckPlan,
    loaded_config: &'a LoadedConfig,
    color_mode:    ColorMode,
    output_format: OutputFormat,
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

impl FixScans<'_> {
    fn import_fix_notice_count(self) -> Option<usize> {
        [
            self.imports.map(|scan| scan.findings.len()),
            self.module_imports.map(|scan| scan.findings.len()),
            self.inline_types.map(|scan| scan.findings.len()),
            self.unused_pub.map(|scan| scan.fixes.len()),
            self.narrowed_pub.map(|scan| scan.fixes.len()),
            self.field_visibility.map(|scan| scan.fixes.len()),
            self.imports_at_top.map(|scan| scan.findings.len()),
        ]
        .into_iter()
        .flatten()
        .reduce(|total, count| total + count)
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

    fn plan(&self, operation_mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let output_mode = if self.output_format == OutputFormat::Json {
            BuildOutputMode::Json
        } else if operation_mode.fixes.contains(FixKind::PubUse) {
            BuildOutputMode::SuppressUnusedImportWarnings
        } else {
            BuildOutputMode::Full
        };
        let selection_result = self.build_selection(output_mode)?;
        let report = selection_result.report;
        let check_duration = selection_result.check_duration;
        let compiler_warnings = selection_result.compiler_warnings;
        let compiler_fixable = selection_result.compiler_fixable;
        let diagnostics_config = &self.loaded_config.diagnostics_config;
        let import_scan = (operation_mode.fixes.contains(FixKind::ShortenImport)
            && (diagnostics_config.is_enabled(DiagnosticCode::ShortenLocalCrateImport)
                == DiagnosticStatus::Enabled
                || diagnostics_config.is_enabled(DiagnosticCode::ReplaceDeepSuperImport)
                    == DiagnosticStatus::Enabled))
            .then(|| imports::scan_selection(self.selection))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let prefer_module_import_scan =
            (operation_mode.fixes.contains(FixKind::PreferModuleImport)
                && diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport)
                    == DiagnosticStatus::Enabled)
                .then(|| prefer_module_import::scan_selection(self.selection))
                .transpose()
                .map_err(MendFailure::Unexpected)?;
        let inline_path_scan = (operation_mode
            .fixes
            .contains(FixKind::InlinePathQualifiedType)
            && diagnostics_config.is_enabled(DiagnosticCode::InlinePathQualifiedType)
                == DiagnosticStatus::Enabled)
            .then(|| inline_path_qualified_type::scan_selection(self.selection))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let narrow_pub_crate_scan = (operation_mode.fixes.contains(FixKind::NarrowToPubCrate)
            && diagnostics_config.is_enabled(DiagnosticCode::NarrowToPubCrate)
                == DiagnosticStatus::Enabled)
            .then(|| narrow_pub_crate::scan_from_report(&report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let unused_pub_scan = (operation_mode.fixes.contains(FixKind::UnusedPub)
            && diagnostics_config.is_enabled(DiagnosticCode::UnusedPub)
                == DiagnosticStatus::Enabled)
            .then(|| unused_pub::scan_from_report(&report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let field_visibility_fix_scan = (operation_mode.fixes.contains(FixKind::FieldVisibility)
            && diagnostics_config.is_enabled(DiagnosticCode::FieldVisibilityWiderThanType)
                == DiagnosticStatus::Enabled)
            .then(|| field_visibility::scan_from_report(&report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let imports_at_top_scan = (operation_mode.fixes.contains(FixKind::ImportsAtTop)
            && diagnostics_config.is_enabled(DiagnosticCode::ImportsAtTop)
                == DiagnosticStatus::Enabled)
            .then(|| imports_at_top::scan_selection(self.selection))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let pub_use_scan = operation_mode
            .fixes
            .contains(FixKind::PubUse)
            .then(|| pub_use_fixes::scan_selection(self.selection, &report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;

        Ok(RunPlan {
            operation_mode,
            report,
            import_scan,
            prefer_module_import_scan,
            inline_path_scan,
            unused_pub_scan,
            narrow_pub_crate_scan,
            field_visibility_fix_scan,
            imports_at_top_scan,
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

    fn apply(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let plan_check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        let fix_scans = planned.fix_scans();
        let applied_pub_use = fix_scans.pub_use.map_or(0, |scan| scan.applied);
        let fixes = Self::combined_fixes(fix_scans)?;
        if fixes.is_empty() {
            let notice = Self::build_fix_notice(
                planned.operation_mode.intent,
                Some(&planned.report),
                fix_scans,
            );
            let warning_facts = planned.report.facts.compiler_warning_facts;
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
                check_duration: plan_check_duration,
                compiler_warnings,
                compiler_fixable,
                applied_pub_use: 0,
                compiler_warning_facts: warning_facts,
            });
        }

        let snapshots = imports::snapshot_files(&fixes).map_err(MendFailure::Unexpected)?;
        imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        let validation_output_mode = if self.output_format == OutputFormat::Json {
            BuildOutputMode::Json
        } else {
            BuildOutputMode::Quiet
        };
        match self.build_selection(validation_output_mode) {
            Ok(validation) => {
                let check_duration = plan_check_duration + validation.check_duration;
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&validation.report),
                    fix_scans,
                );
                let warning_facts = validation.report.facts.compiler_warning_facts;
                Ok(ExecutionOutcome {
                    report: validation.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use,
                    compiler_warning_facts: warning_facts,
                })
            },
            Err(err) => {
                let rollback_status = match imports::restore_files(&snapshots) {
                    Ok(()) => RollbackStatus::Restored,
                    Err(_) => RollbackStatus::RestoreFailed,
                };
                let cause = match err {
                    MendFailure::Analysis(a) => a.cause,
                    MendFailure::Unexpected(e) => CompilerFailureCause::Unexpected(e),
                    MendFailure::FixValidation(f) => f.cause,
                };
                Err(MendFailure::FixValidation(FixValidationFailure {
                    rollback_status,
                    cause,
                }))
            },
        }
    }

    fn combined_fixes(fix_scans: FixScans<'_>) -> Result<ValidatedFixSet, MendFailure> {
        // Collect `prefer_module_import` fix ranges for deconfliction with `ShortenImport`
        let prefer_ranges: Vec<(&Path, usize, usize)> = fix_scans
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
        if let Some(scan) = fix_scans.unused_pub {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.narrowed_pub {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.field_visibility {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.imports_at_top {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.pub_use {
            fixes.extend(scan.fixes.iter().cloned());
        }

        let fixes = drop_conflicting_import_groups(fixes);

        imports::ValidatedFixSet::try_from(fixes).map_err(MendFailure::Unexpected)
    }

    fn build_fix_notice(
        intent: OperationIntent,
        report: Option<&Report>,
        fix_scans: FixScans<'_>,
    ) -> Option<ExecutionNotice> {
        let mut notices = Vec::new();
        if let Some(import_fix_count) = fix_scans.import_fix_notice_count() {
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

        // The historical `ImportCleanupSuggested` notice is gone; the
        // orchestrator runs `cargo fix` automatically when `--fix-pub-use`
        // applied edits and `unused import` warnings followed.
        let _ = report;

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

/// Cross-pass import reservation.
///
/// Each pass tags its "`use X;` insertion + the rewrites that depend on
/// it" with a shared [`imports::ImportGroup`] (bare name + full path). If
/// two different passes each want to bring a *different* full path into
/// scope under the *same* bare name within the same file, applying both
/// would either produce duplicate-name errors or silently shadow an
/// existing binding. The safe default is to drop every fix in any
/// conflicting group so the file either compiles after the remaining
/// fixes land or is left for the user to reconcile. Untagged fixes
/// (`import_group: None`) pass through unchanged.
fn drop_conflicting_import_groups(fixes: Vec<imports::UseFix>) -> Vec<imports::UseFix> {
    let mut bare_name_to_paths: BTreeMap<(PathBuf, String), BTreeSet<String>> = BTreeMap::new();
    for fix in &fixes {
        if let Some(group) = &fix.import_group {
            bare_name_to_paths
                .entry((fix.path.clone(), group.bare_name.clone()))
                .or_default()
                .insert(group.full_path.clone());
        }
    }

    let conflicting: BTreeSet<(PathBuf, String)> = bare_name_to_paths
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(key, _)| key)
        .collect();

    if conflicting.is_empty() {
        return fixes;
    }

    fixes
        .into_iter()
        .filter(|fix| {
            fix.import_group.as_ref().is_none_or(|group| {
                !conflicting.contains(&(fix.path.clone(), group.bare_name.clone()))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::FieldVisibilityFixScan;
    use super::FixScans;
    use super::MendRunner;
    use super::drop_conflicting_import_groups;
    use super::imports::ImportGroup;
    use super::imports::UseFix;
    use crate::config::OperationIntent;

    fn tagged(path: &str, start: usize, replacement: &str, bare: &str, full: &str) -> UseFix {
        UseFix {
            path: PathBuf::from(path),
            start,
            end: start,
            replacement: replacement.to_string(),
            import_group: Some(ImportGroup {
                bare_name: bare.to_string(),
                full_path: full.to_string(),
            }),
        }
    }

    fn untagged(path: &str, start: usize, replacement: &str) -> UseFix {
        UseFix {
            path: PathBuf::from(path),
            start,
            end: start,
            replacement: replacement.to_string(),
            import_group: None,
        }
    }

    fn fix_scans_with_field_visibility(field_visibility: &FieldVisibilityFixScan) -> FixScans<'_> {
        FixScans {
            imports:          None,
            module_imports:   None,
            inline_types:     None,
            unused_pub:       None,
            narrowed_pub:     None,
            field_visibility: Some(field_visibility),
            imports_at_top:   None,
            pub_use:          None,
        }
    }

    fn field_visibility_scan(fixes: Vec<UseFix>) -> FieldVisibilityFixScan {
        FieldVisibilityFixScan { fixes }
    }

    fn field_visibility_fix() -> UseFix { untagged("src/lib.rs", 10, "") }

    #[test]
    fn field_visibility_scan_emits_import_fix_notice() {
        let field_visibility = field_visibility_scan(vec![field_visibility_fix()]);
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: applied 1 import fix(es)".to_string())
        );
    }

    #[test]
    fn empty_field_visibility_scan_emits_noop_import_fix_notice() {
        let field_visibility = field_visibility_scan(Vec::new());
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: no import fixes available".to_string())
        );
    }

    #[test]
    fn no_conflicts_pass_through_unchanged() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::foo::Bar;\n",
                "Bar",
                "crate::foo::Bar",
            ),
            tagged("src/a.rs", 50, "Bar", "Bar", "crate::foo::Bar"),
            tagged(
                "src/a.rs",
                0,
                "use crate::foo::Baz;\n",
                "Baz",
                "crate::foo::Baz",
            ),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn same_bare_name_different_paths_drops_all_tagged() {
        // Two passes both want to introduce `Package` but from different paths.
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged("src/a.rs", 50, "Package", "Package", "crate::a::Package"),
            tagged(
                "src/a.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
            tagged("src/a.rs", 75, "Package", "Package", "crate::b::Package"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert!(
            result.is_empty(),
            "conflicting-group fixes should all be dropped, got {result:?}"
        );
    }

    #[test]
    fn same_bare_name_same_full_path_kept() {
        // Same group identity across fixes — not a conflict.
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged("src/a.rs", 50, "Package", "Package", "crate::a::Package"),
            tagged("src/a.rs", 80, "Package", "Package", "crate::a::Package"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn conflict_isolated_per_file() {
        // Same bare name, different paths — but in different files. No conflict.
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged(
                "src/b.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn untagged_fixes_always_pass_through_even_with_conflicts() {
        // A conflict on `Package` should drop tagged fixes but leave an
        // unrelated `ShortenImport`-style untagged fix intact.
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged(
                "src/a.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
            untagged("src/a.rs", 100, "use super::other;"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 1);
        assert!(result[0].import_group.is_none());
    }
}

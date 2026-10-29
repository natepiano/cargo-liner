use super::CheckedSelection;
use super::MendRunner;
use super::RunPlan;
use crate::compiler::BuildOutputMode;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::FixKind;
use crate::config::FixKindSelection;
use crate::config::OperationMode;
use crate::fixes::field_visibility;
use crate::fixes::narrow_pub_crate;
use crate::fixes::pub_use_fixes;
use crate::fixes::pub_use_fixes::PubUseFixScan;
use crate::fixes::restricted_annotation;
use crate::fixes::restricted_annotation::RestrictedAnnotationScan;
use crate::fixes::subtree_reexport;
use crate::fixes::subtree_reexport::SubtreeReexportScan;
use crate::fixes::unused_pub;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;
use crate::reporting::Report;
use crate::rust_syntax::ParsedSources;

enum PassOutputPolicy {
    Json,
    PubUseText,
    FullText,
}

impl PassOutputPolicy {
    const fn for_pass(format: OutputFormat, pub_use: FixKindSelection) -> Self {
        match (format, pub_use) {
            (OutputFormat::Json, FixKindSelection::Selected | FixKindSelection::NotSelected) => {
                Self::Json
            },
            (OutputFormat::Human, FixKindSelection::Selected) => Self::PubUseText,
            (OutputFormat::Human, FixKindSelection::NotSelected) => Self::FullText,
        }
    }
}

impl From<PassOutputPolicy> for BuildOutputMode {
    fn from(policy: PassOutputPolicy) -> Self {
        match policy {
            PassOutputPolicy::Json => Self::Json,
            PassOutputPolicy::PubUseText => Self::SuppressUnusedImportWarnings,
            PassOutputPolicy::FullText => Self::Full,
        }
    }
}

impl MendRunner<'_> {
    pub(super) fn plan(&self, operation_mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let pub_use = operation_mode.fixes.selection(FixKind::PubUse);
        let output_mode =
            BuildOutputMode::from(PassOutputPolicy::for_pass(self.output_format, pub_use));
        let checked = self.build_selection(output_mode)?;
        self.scan_fixes(operation_mode, checked, pub_use)
    }

    /// Runs every fixer `operation_mode` enables against the checked
    /// selection, reusing its syntax scans and the sources they read.
    fn scan_fixes(
        &self,
        operation_mode: OperationMode,
        checked: CheckedSelection,
        pub_use: FixKindSelection,
    ) -> Result<RunPlan, MendFailure> {
        let CheckedSelection {
            result: selection_result,
            scans,
            sources,
        } = checked;
        let report = selection_result.report;
        let check_duration = selection_result.check_duration;
        let compiler_warnings = selection_result.compiler_warnings;
        let compiler_fixable = selection_result.compiler_fixable;
        let enabled = |fix_kind: FixKind, codes: &[DiagnosticCode]| {
            self.fix_selection(&operation_mode, fix_kind, codes)
        };
        let prefer_module_import_scan = scans.prefer_module_import.filter(|_| {
            match enabled(
                FixKind::PreferModuleImport,
                &[DiagnosticCode::PreferModuleImport],
            ) {
                FixKindSelection::Selected => true,
                FixKindSelection::NotSelected => false,
            }
        });
        let inline_path_scan = scans.inline_path.filter(|_| {
            match enabled(
                FixKind::InlinePathQualifiedType,
                &[DiagnosticCode::InlinePathQualifiedType],
            ) {
                FixKindSelection::Selected => true,
                FixKindSelection::NotSelected => false,
            }
        });
        let import_scan = scans.imports.filter(|_| {
            match enabled(
                FixKind::ShortenImport,
                &[
                    DiagnosticCode::ShortenLocalCrateImport,
                    DiagnosticCode::ReplaceDeepSuperImport,
                ],
            ) {
                FixKindSelection::Selected => true,
                FixKindSelection::NotSelected => false,
            }
        });
        let narrow_pub_crate_scan = match enabled(
            FixKind::NarrowToPubCrate,
            &[DiagnosticCode::NarrowToPubCrate],
        ) {
            FixKindSelection::Selected => {
                Some(narrow_pub_crate::scan_from_report(&report).map_err(MendFailure::Unexpected)?)
            },
            FixKindSelection::NotSelected => None,
        };
        let restricted_annotation_scan =
            self.scan_restricted_annotations(&operation_mode, &report)?;
        let unused_pub_scan = match enabled(FixKind::UnusedPub, &[DiagnosticCode::UnusedPub]) {
            FixKindSelection::Selected => {
                Some(unused_pub::scan_from_report(&report).map_err(MendFailure::Unexpected)?)
            },
            FixKindSelection::NotSelected => None,
        };
        let field_visibility_fix_scan = match enabled(
            FixKind::FieldVisibility,
            &[DiagnosticCode::FieldVisibilityWiderThanType],
        ) {
            FixKindSelection::Selected => {
                Some(field_visibility::scan_from_report(&report).map_err(MendFailure::Unexpected)?)
            },
            FixKindSelection::NotSelected => None,
        };
        let imports_at_top_scan = scans.imports_at_top.filter(|_| {
            match enabled(FixKind::ImportsAtTop, &[DiagnosticCode::ImportsAtTop]) {
                FixKindSelection::Selected => true,
                FixKindSelection::NotSelected => false,
            }
        });
        let pub_use_scan = self.scan_parent_facades(pub_use, &report, &sources)?;
        let subtree_reexport_scan =
            self.scan_subtree_reexports(&operation_mode, &report, &sources)?;

        Ok(RunPlan {
            operation_mode,
            report,
            import_scan,
            prefer_module_import_scan,
            inline_path_scan,
            unused_pub_scan,
            narrow_pub_crate_scan,
            restricted_annotation_scan,
            field_visibility_fix_scan,
            imports_at_top_scan,
            pub_use_scan,
            subtree_reexport_scan,
            check_duration,
            compiler_warnings,
            compiler_fixable,
        })
    }

    fn scan_restricted_annotations(
        &self,
        operation_mode: &OperationMode,
        report: &Report,
    ) -> Result<Option<RestrictedAnnotationScan>, MendFailure> {
        match self.fix_selection(
            operation_mode,
            FixKind::RestrictedAnnotation,
            &[
                DiagnosticCode::OverbroadPubCrate,
                DiagnosticCode::ForbiddenPubInCrate,
                DiagnosticCode::SuspiciousPub,
            ],
        ) {
            FixKindSelection::Selected => restricted_annotation::scan_from_report(report)
                .map(Some)
                .map_err(MendFailure::Unexpected),
            FixKindSelection::NotSelected => Ok(None),
        }
    }

    fn scan_parent_facades(
        &self,
        pub_use: FixKindSelection,
        report: &Report,
        sources: &ParsedSources,
    ) -> Result<Option<PubUseFixScan>, MendFailure> {
        match pub_use {
            FixKindSelection::Selected => {
                pub_use_fixes::scan_selection(self.selection, report, sources)
                    .map(Some)
                    .map_err(MendFailure::Unexpected)
            },
            FixKindSelection::NotSelected => Ok(None),
        }
    }

    fn scan_subtree_reexports(
        &self,
        operation_mode: &OperationMode,
        report: &Report,
        sources: &ParsedSources,
    ) -> Result<Option<SubtreeReexportScan>, MendFailure> {
        match self.fix_selection(
            operation_mode,
            FixKind::PubUseOutsideSubtree,
            &[DiagnosticCode::PubUseOutsideSubtree],
        ) {
            FixKindSelection::Selected => {
                subtree_reexport::scan_selection(self.selection, report, sources)
                    .map(Some)
                    .map_err(MendFailure::Unexpected)
            },
            FixKindSelection::NotSelected => Ok(None),
        }
    }

    /// Selects `fix_kind` for this pass only when the mode requests it and at least
    /// one diagnostic it fixes is enabled.
    fn fix_selection(
        &self,
        operation_mode: &OperationMode,
        fix_kind: FixKind,
        codes: &[DiagnosticCode],
    ) -> FixKindSelection {
        match operation_mode.fixes.selection(fix_kind) {
            FixKindSelection::NotSelected => FixKindSelection::NotSelected,
            FixKindSelection::Selected => {
                if codes.iter().any(|&code| {
                    self.loaded_config.diagnostics_config.is_enabled(code)
                        == DiagnosticStatus::Enabled
                }) {
                    FixKindSelection::Selected
                } else {
                    FixKindSelection::NotSelected
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PassOutputPolicy;
    use crate::compiler::BuildOutputMode;
    use crate::config::FixKindSelection;
    use crate::reporting::OutputFormat;

    #[test]
    fn json_output_wins_over_pub_use_warning_suppression() {
        for activation in [FixKindSelection::Selected, FixKindSelection::NotSelected] {
            assert!(matches!(
                BuildOutputMode::from(PassOutputPolicy::for_pass(OutputFormat::Json, activation)),
                BuildOutputMode::Json
            ));
        }
        assert!(matches!(
            BuildOutputMode::from(PassOutputPolicy::for_pass(
                OutputFormat::Human,
                FixKindSelection::Selected
            )),
            BuildOutputMode::SuppressUnusedImportWarnings
        ));
        assert!(matches!(
            BuildOutputMode::from(PassOutputPolicy::for_pass(
                OutputFormat::Human,
                FixKindSelection::NotSelected
            )),
            BuildOutputMode::Full
        ));
    }
}

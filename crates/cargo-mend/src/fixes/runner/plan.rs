use super::CheckedSelection;
use super::MendRunner;
use super::RunPlan;
use crate::compiler::BuildOutputMode;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::FixKind;
use crate::config::OperationMode;
use crate::fixes::field_visibility;
use crate::fixes::narrow_pub_crate;
use crate::fixes::pub_use_fixes;
use crate::fixes::restricted_annotation;
use crate::fixes::subtree_reexport;
use crate::fixes::unused_pub;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;

#[derive(Clone, Copy)]
enum PubUseActivation {
    Enabled,
    Disabled,
}

impl From<bool> for PubUseActivation {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

enum PassOutputPolicy {
    Json,
    PubUseText,
    FullText,
}

impl PassOutputPolicy {
    const fn for_pass(format: OutputFormat, pub_use: PubUseActivation) -> Self {
        match (format, pub_use) {
            (OutputFormat::Json, _) => Self::Json,
            (OutputFormat::Human, PubUseActivation::Enabled) => Self::PubUseText,
            (OutputFormat::Human, PubUseActivation::Disabled) => Self::FullText,
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
        let pub_use = PubUseActivation::from(operation_mode.fixes.contains(FixKind::PubUse));
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
        pub_use: PubUseActivation,
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
            self.fix_enabled(&operation_mode, fix_kind, codes)
        };
        let prefer_module_import_scan = scans.prefer_module_import.filter(|_| {
            enabled(
                FixKind::PreferModuleImport,
                &[DiagnosticCode::PreferModuleImport],
            )
        });
        let inline_path_scan = scans.inline_path.filter(|_| {
            enabled(
                FixKind::InlinePathQualifiedType,
                &[DiagnosticCode::InlinePathQualifiedType],
            )
        });
        let import_scan = scans.imports.filter(|_| {
            enabled(
                FixKind::ShortenImport,
                &[
                    DiagnosticCode::ShortenLocalCrateImport,
                    DiagnosticCode::ReplaceDeepSuperImport,
                ],
            )
        });
        let narrow_pub_crate_scan = enabled(
            FixKind::NarrowToPubCrate,
            &[DiagnosticCode::NarrowToPubCrate],
        )
        .then(|| narrow_pub_crate::scan_from_report(&report))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let restricted_annotation_scan = enabled(
            FixKind::RestrictedAnnotation,
            &[
                DiagnosticCode::OverbroadPubCrate,
                DiagnosticCode::ForbiddenPubInCrate,
                DiagnosticCode::SuspiciousPub,
            ],
        )
        .then(|| restricted_annotation::scan_from_report(&report))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let unused_pub_scan = enabled(FixKind::UnusedPub, &[DiagnosticCode::UnusedPub])
            .then(|| unused_pub::scan_from_report(&report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let field_visibility_fix_scan = enabled(
            FixKind::FieldVisibility,
            &[DiagnosticCode::FieldVisibilityWiderThanType],
        )
        .then(|| field_visibility::scan_from_report(&report))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let imports_at_top_scan = scans
            .imports_at_top
            .filter(|_| enabled(FixKind::ImportsAtTop, &[DiagnosticCode::ImportsAtTop]));
        let pub_use_scan = match pub_use {
            PubUseActivation::Enabled => Some(
                pub_use_fixes::scan_selection(self.selection, &report, &sources)
                    .map_err(MendFailure::Unexpected)?,
            ),
            PubUseActivation::Disabled => None,
        };
        let subtree_reexport_scan = enabled(
            FixKind::PubUseOutsideSubtree,
            &[DiagnosticCode::PubUseOutsideSubtree],
        )
        .then(|| subtree_reexport::scan_selection(self.selection, &report, &sources))
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

    /// Whether `fix_kind` runs this pass: the mode requests it and at least
    /// one diagnostic it fixes is enabled.
    fn fix_enabled(
        &self,
        operation_mode: &OperationMode,
        fix_kind: FixKind,
        codes: &[DiagnosticCode],
    ) -> bool {
        operation_mode.fixes.contains(fix_kind)
            && codes.iter().any(|&code| {
                self.loaded_config.diagnostics_config.is_enabled(code) == DiagnosticStatus::Enabled
            })
    }
}

#[cfg(test)]
mod tests {
    use super::PassOutputPolicy;
    use super::PubUseActivation;
    use crate::compiler::BuildOutputMode;
    use crate::reporting::OutputFormat;

    #[test]
    fn json_output_wins_over_pub_use_warning_suppression() {
        for activation in [PubUseActivation::Enabled, PubUseActivation::Disabled] {
            assert!(matches!(
                BuildOutputMode::from(PassOutputPolicy::for_pass(OutputFormat::Json, activation)),
                BuildOutputMode::Json
            ));
        }
        assert!(matches!(
            BuildOutputMode::from(PassOutputPolicy::for_pass(
                OutputFormat::Human,
                PubUseActivation::Enabled
            )),
            BuildOutputMode::SuppressUnusedImportWarnings
        ));
        assert!(matches!(
            BuildOutputMode::from(PassOutputPolicy::for_pass(
                OutputFormat::Human,
                PubUseActivation::Disabled
            )),
            BuildOutputMode::Full
        ));
    }
}

use super::MendRunner;
use super::RunPlan;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::FixKind;
use crate::config::OperationMode;
use crate::fixes::field_visibility;
use crate::fixes::imports;
use crate::fixes::imports_at_top;
use crate::fixes::inline_path_qualified_type;
use crate::fixes::narrow_pub_crate;
use crate::fixes::prefer_module_import;
use crate::fixes::pub_use_fixes;
use crate::fixes::restricted_annotation;
use crate::fixes::subtree_reexport;
use crate::fixes::unused_pub;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;

impl MendRunner<'_> {
    pub(super) fn plan(&self, operation_mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let output_mode = if self.output_format == OutputFormat::Json {
            BuildOutputMode::Json
        } else if operation_mode.fixes.contains(FixKind::PubUse) {
            BuildOutputMode::SuppressUnusedImportWarnings
        } else {
            BuildOutputMode::Full
        };
        let selection_result = self.build_selection(output_mode)?;
        self.scan_fixes(operation_mode, selection_result)
    }

    /// Runs every fixer `operation_mode` enables against the checked
    /// selection.
    fn scan_fixes(
        &self,
        operation_mode: OperationMode,
        selection_result: SelectionResult,
    ) -> Result<RunPlan, MendFailure> {
        let report = selection_result.report;
        let check_duration = selection_result.check_duration;
        let compiler_warnings = selection_result.compiler_warnings;
        let compiler_fixable = selection_result.compiler_fixable;
        let enabled = |fix_kind: FixKind, codes: &[DiagnosticCode]| {
            self.fix_enabled(&operation_mode, fix_kind, codes)
        };
        let import_scan = enabled(
            FixKind::ShortenImport,
            &[
                DiagnosticCode::ShortenLocalCrateImport,
                DiagnosticCode::ReplaceDeepSuperImport,
            ],
        )
        .then(|| imports::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let prefer_module_import_scan = enabled(
            FixKind::PreferModuleImport,
            &[DiagnosticCode::PreferModuleImport],
        )
        .then(|| prefer_module_import::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
        let inline_path_scan = enabled(
            FixKind::InlinePathQualifiedType,
            &[DiagnosticCode::InlinePathQualifiedType],
        )
        .then(|| inline_path_qualified_type::scan_selection(self.selection))
        .transpose()
        .map_err(MendFailure::Unexpected)?;
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
        let imports_at_top_scan = enabled(FixKind::ImportsAtTop, &[DiagnosticCode::ImportsAtTop])
            .then(|| imports_at_top::scan_selection(self.selection))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let pub_use_scan = operation_mode
            .fixes
            .contains(FixKind::PubUse)
            .then(|| pub_use_fixes::scan_selection(self.selection, &report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let subtree_reexport_scan = enabled(
            FixKind::PubUseOutsideSubtree,
            &[DiagnosticCode::PubUseOutsideSubtree],
        )
        .then(|| subtree_reexport::scan_selection(self.selection, &report))
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

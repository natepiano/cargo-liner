use super::MendRunner;
use super::RunPlan;
use crate::compiler::BuildOutputMode;
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
}

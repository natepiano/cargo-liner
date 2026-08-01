use std::time::Duration;

use anyhow::Result;

use crate::config::LoadedConfig;
use crate::config::OperationMode;
use crate::fixes::field_visibility::FieldVisibilityFixScan;
use crate::fixes::imports::ImportScan;
use crate::fixes::imports_at_top::ImportsAtTopScan;
use crate::fixes::inline_path_qualified_type::InlinePathScan;
use crate::fixes::narrow_pub_crate::NarrowPubCrateScan;
use crate::fixes::prefer_module_import::PreferModuleImportScan;
use crate::fixes::pub_use_fixes::PubUseFixScan;
use crate::fixes::unused_pub::UnusedPubScan;
use crate::reporting::ColorMode;
use crate::reporting::ExecutionOutcome;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;
use crate::reporting::Report;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

pub(crate) struct MendRunner<'a> {
    pub(super) selection:     &'a Selection,
    pub(super) cargo_plan:    &'a CargoCheckPlan,
    pub(super) loaded_config: &'a LoadedConfig,
    pub(super) color_mode:    ColorMode,
    pub(super) output_format: OutputFormat,
}

impl<'a> MendRunner<'a> {
    pub const fn new(
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

    pub fn run(&self, operation_mode: OperationMode) -> Result<ExecutionOutcome, MendFailure> {
        let planned = self.plan(operation_mode)?;
        self.execute(planned)
    }
}

pub(super) struct RunPlan {
    pub(super) operation_mode:            OperationMode,
    pub(super) report:                    Report,
    pub(super) import_scan:               Option<ImportScan>,
    pub(super) prefer_module_import_scan: Option<PreferModuleImportScan>,
    pub(super) inline_path_scan:          Option<InlinePathScan>,
    pub(super) unused_pub_scan:           Option<UnusedPubScan>,
    pub(super) narrow_pub_crate_scan:     Option<NarrowPubCrateScan>,
    pub(super) field_visibility_fix_scan: Option<FieldVisibilityFixScan>,
    pub(super) imports_at_top_scan:       Option<ImportsAtTopScan>,
    pub(super) pub_use_scan:              Option<PubUseFixScan>,
    pub(super) check_duration:            Duration,
    pub(super) compiler_warnings:         usize,
    pub(super) compiler_fixable:          usize,
}

#[derive(Clone, Copy)]
pub(super) struct FixScans<'a> {
    pub(super) imports:          Option<&'a ImportScan>,
    pub(super) module_imports:   Option<&'a PreferModuleImportScan>,
    pub(super) inline_types:     Option<&'a InlinePathScan>,
    pub(super) unused_pub:       Option<&'a UnusedPubScan>,
    pub(super) narrowed_pub:     Option<&'a NarrowPubCrateScan>,
    pub(super) field_visibility: Option<&'a FieldVisibilityFixScan>,
    pub(super) imports_at_top:   Option<&'a ImportsAtTopScan>,
    pub(super) pub_use:          Option<&'a PubUseFixScan>,
}

impl RunPlan {
    pub(super) const fn fix_scans(&self) -> FixScans<'_> {
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

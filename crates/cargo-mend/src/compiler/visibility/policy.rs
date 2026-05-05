use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Result;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::scan::AllowanceReason;
use super::scan::CrateKind;
use super::scan::ModuleLocation;
use super::scan::ParentVisibility;
use super::scan::SuspiciousPubAssessment;
use super::scan::SuspiciousPubInput;
use super::scan::VisibilityContext;
use crate::compiler::exposure;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeFixSupport;
use crate::compiler::facade::ParentFacadeUsage;
use crate::compiler::facade::ParentFacadeVisibility;
use crate::constants::RUST_LIB_FILE;
use crate::constants::RUST_MODULE_FILE;
use crate::constants::SOURCE_DIR_BENCHES;
use crate::constants::SOURCE_DIR_EXAMPLES;
use crate::constants::SOURCE_DIR_TESTS;
use crate::fix_support::FixSupport;

pub(super) fn classify_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
) -> Result<SuspiciousPubAssessment> {
    if let Some(allowance) = basic_suspicious_pub_allowance(
        ctx,
        input.def_id,
        input.config_rel_path,
        input.parent_visibility,
        input.name,
    ) {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let parent_facade_export = input
        .name
        .map(|name| {
            facade::parent_facade_export_status(
                ctx.source_cache,
                ctx.settings,
                ctx.source_root,
                input.file_path,
                name,
            )
        })
        .transpose()?
        .flatten();

    if let Some(assessment) = assess_parent_facade_usage(parent_facade_export.as_ref()) {
        return Ok(assessment);
    }

    if let Some(allowance) = assess_signature_exposure_allowance(ctx, input.file_path, input.name)?
    {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let stale_result = parent_facade_export.as_ref().and_then(|status| {
        let message = match status.usage {
            ParentFacadeUsage::Unused => format!(
                "parent module also has an `unused import` warning for this `pub use` at {}:{}",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
            | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport => format!(
                "parent `pub use` at {}:{} is only used through crate-relative paths inside its own subtree",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport
            | ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
            | ParentFacadeUsage::UsedOutsideParentSubtree => return None,
        };
        Some((message, status))
    });

    if matches!(input.module_location, ModuleLocation::ShallowPrivateModule)
        && stale_result.is_none()
    {
        return Ok(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ShallowPrivateModulePolicy,
        ));
    }

    let (related, fixability, stale_parent_pub_use) = match stale_result {
        Some((message, status)) => {
            let fixability = if status.fix_supported == ParentFacadeFixSupport::Supported {
                FixSupport::FixPubUse
            } else {
                FixSupport::NeedsManualPubUseCleanup
            };
            (Some(message), fixability, Some(status.clone()))
        },
        None => (None, FixSupport::None, None),
    };

    Ok(SuspiciousPubAssessment::Warn {
        fixability,
        related,
        stale_parent_pub_use,
    })
}

// Items at depth 1 (`crate::foo`) and depth 2 (`crate::foo::bar`) both map to
// `ShallowPrivateModule`. Depth 2 covers the common `src/<top>/<child>.rs`
// library layout: when the top-level module is private, nothing outside its
// subtree can reach the child regardless of `pub(crate)` vs `pub(super)`, so
// the policy treats them the same as depth-1 items. Depth 3+ falls through to
// `NestedModule`, where `pub(crate)` can meaningfully widen reach beyond what
// `pub(super)` would allow.
pub(super) fn resolve_module_location(tcx: TyCtxt<'_>, parent_def: LocalDefId) -> ModuleLocation {
    if parent_def == CRATE_DEF_ID {
        return ModuleLocation::CrateRoot;
    }

    let grandparent = tcx.parent_module_from_def_id(parent_def).to_local_def_id();
    if grandparent == CRATE_DEF_ID {
        return ModuleLocation::ShallowPrivateModule;
    }

    let great_grandparent = tcx.parent_module_from_def_id(grandparent).to_local_def_id();
    if great_grandparent == CRATE_DEF_ID {
        return ModuleLocation::ShallowPrivateModule;
    }

    ModuleLocation::NestedModule
}

pub(super) const fn allow_pub_crate_by_policy(
    crate_kind: CrateKind,
    module_location: ModuleLocation,
    parent_visibility: ParentVisibility,
) -> bool {
    match (crate_kind, module_location) {
        (CrateKind::Library, ModuleLocation::CrateRoot) => true,
        (CrateKind::IntegrationTest, _) => false,
        (_, ModuleLocation::ShallowPrivateModule) => {
            matches!(parent_visibility, ParentVisibility::Private)
        },
        _ => false,
    }
}

pub(super) fn crate_kind_for_root(root_module: &Path, package_root: &Path) -> CrateKind {
    if root_module.file_name().and_then(OsStr::to_str) == Some(RUST_LIB_FILE) {
        return CrateKind::Library;
    }
    let canonical_root =
        fs::canonicalize(root_module).unwrap_or_else(|_| root_module.to_path_buf());
    let canonical_package =
        fs::canonicalize(package_root).unwrap_or_else(|_| package_root.to_path_buf());
    let Ok(relative) = canonical_root.strip_prefix(&canonical_package) else {
        return CrateKind::Binary;
    };
    let components: Vec<_> = relative.components().collect();
    match components.as_slice() {
        [first, _]
            if matches!(
                first.as_os_str().to_str(),
                Some(SOURCE_DIR_TESTS | SOURCE_DIR_EXAMPLES | SOURCE_DIR_BENCHES)
            ) =>
        {
            CrateKind::IntegrationTest
        },
        _ => CrateKind::Binary,
    }
}

pub(super) const fn forbidden_pub_crate_help(module_location: ModuleLocation) -> &'static str {
    if matches!(
        module_location,
        ModuleLocation::CrateRoot | ModuleLocation::ShallowPrivateModule
    ) {
        "consider using just `pub` or removing `pub(crate)` entirely"
    } else {
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    }
}

pub(super) fn is_top_level_module_file(
    source_root: &Path,
    root_module: &Path,
    file: &Path,
) -> bool {
    if file == root_module {
        return false;
    }
    let Ok(relative) = file.strip_prefix(source_root) else {
        return false;
    };
    let count = relative.components().count();
    if count == 1 {
        return true;
    }
    count == 2 && relative.file_name().and_then(OsStr::to_str) == Some(RUST_MODULE_FILE)
}

pub(super) fn is_boundary_file(source_root: &Path, root_module: &Path, file: &Path) -> bool {
    let is_root_file = file == root_module;
    let is_module_rs = file.file_name().and_then(OsStr::to_str) == Some(RUST_MODULE_FILE);
    let is_top_level_file = file
        .strip_prefix(source_root)
        .ok()
        .is_some_and(|path| path.components().count() == 1);
    is_root_file || is_module_rs || is_top_level_file
}

pub(super) fn suspicious_pub_note(crate_kind: CrateKind, kind_label: &str) -> String {
    match crate_kind {
        CrateKind::Library => {
            format!("{kind_label} is not reachable from the crate's public API")
        },
        CrateKind::Binary | CrateKind::IntegrationTest => {
            format!("{kind_label} is not used outside its parent module subtree")
        },
    }
}

fn basic_suspicious_pub_allowance(
    ctx: &VisibilityContext<'_, '_>,
    def_id: LocalDefId,
    config_rel_path: Option<&str>,
    parent_visibility: ParentVisibility,
    item_name: Option<&str>,
) -> Option<AllowanceReason> {
    let item_key = config_rel_path.and_then(|path| item_name.map(|name| format!("{path}::{name}")));
    let allowlisted = item_key.as_ref().is_some_and(|key| {
        ctx.settings
            .visibility_config
            .allow_pub_items
            .iter()
            .any(|allowed| allowed == key)
    });
    if allowlisted {
        return Some(AllowanceReason::Allowlist);
    }
    if parent_visibility == ParentVisibility::Public {
        return Some(AllowanceReason::ParentIsPublic);
    }
    if ctx
        .effective_visibilities
        .is_public_at_level(def_id, Level::Reachable)
    {
        return Some(AllowanceReason::ReachablePublicApi);
    }
    None
}

fn assess_parent_facade_usage(
    parent_facade_export: Option<&ParentFacadeExportStatus>,
) -> Option<SuspiciousPubAssessment> {
    let status = parent_facade_export?;
    if status.visibility == ParentFacadeVisibility::Super
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::InternalParentFacadeBoundary,
        ));
    }
    match status.usage {
        ParentFacadeUsage::UsedOutsideParentSubtree => Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ParentFacadeUsedOutsideParent,
        )),
        ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport => {
            let related = Some(format!(
                "parent module uses this item as an internal facade at {}:{}",
                status.parent_rel_path, status.parent_line
            ));
            Some(SuspiciousPubAssessment::ReviewInternalParentFacade { related })
        },
        ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport
        | ParentFacadeUsage::Unused => None,
    }
}

fn assess_signature_exposure_allowance(
    ctx: &VisibilityContext<'_, '_>,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<Option<AllowanceReason>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    if exposure::child_item_is_exposed_by_other_crate_visible_signature(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        file_path,
        item_name,
    )? || exposure::impl_item_is_exposed_by_exported_self_type(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        file_path,
        item_name,
    )? || exposure::child_item_is_exposed_by_sibling_boundary_signature(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        file_path,
        item_name,
    )? || exposure::parent_boundary_public_signature_exposes_child_used_outside_parent(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        file_path,
        item_name,
    )? {
        return Ok(Some(AllowanceReason::ExposedByOtherCrateVisibleSignature));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::CrateKind;
    use super::ModuleLocation;
    use super::ParentVisibility;
    use super::allow_pub_crate_by_policy;
    use super::crate_kind_for_root;
    use super::forbidden_pub_crate_help;
    use super::suspicious_pub_note;
    use crate::constants::SOURCE_DIR_BENCHES;
    use crate::constants::SOURCE_DIR_EXAMPLES;
    use crate::constants::SOURCE_DIR_TESTS;

    #[test]
    fn allow_pub_crate_allows_library_crate_root_items() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::CrateRoot,
            ParentVisibility::Public
        ));
    }

    #[test]
    fn allow_pub_crate_allows_shallow_private_library_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::ShallowPrivateModule,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::NestedModule,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_crate_root_items() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::CrateRoot,
            ParentVisibility::Public
        ));
    }

    #[test]
    fn allow_pub_crate_allows_shallow_private_binary_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::ShallowPrivateModule,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::NestedModule,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_integration_test_items_in_any_location() {
        for module_location in [
            ModuleLocation::CrateRoot,
            ModuleLocation::ShallowPrivateModule,
            ModuleLocation::NestedModule,
        ] {
            for parent_visibility in [ParentVisibility::Private, ParentVisibility::Public] {
                assert!(
                    !allow_pub_crate_by_policy(
                        CrateKind::IntegrationTest,
                        module_location,
                        parent_visibility,
                    ),
                    "pub(crate) should be forbidden in integration-test crates \
                     regardless of module location or parent visibility \
                     (location = {module_location:?}, parent = {parent_visibility:?})",
                );
            }
        }
    }

    #[test]
    fn crate_kind_for_root_detects_library_from_lib_rs() {
        let package_root = Path::new("/tmp/pkg");
        assert_eq!(
            crate_kind_for_root(&package_root.join("src/lib.rs"), package_root),
            CrateKind::Library
        );
    }

    #[test]
    fn crate_kind_for_root_detects_binary_from_main_rs() {
        let package_root = Path::new("/tmp/pkg");
        assert_eq!(
            crate_kind_for_root(&package_root.join("src/main.rs"), package_root),
            CrateKind::Binary
        );
    }

    #[test]
    fn crate_kind_for_root_detects_integration_test_roots() {
        let package_root = Path::new("/tmp/pkg");
        for sub in [SOURCE_DIR_TESTS, SOURCE_DIR_EXAMPLES, SOURCE_DIR_BENCHES] {
            let root = package_root.join(sub).join("support.rs");
            assert_eq!(
                crate_kind_for_root(&root, package_root),
                CrateKind::IntegrationTest,
                "{sub}/*.rs should classify as IntegrationTest",
            );
        }
    }

    #[test]
    fn crate_kind_for_root_treats_nested_example_root_as_binary() {
        let package_root = Path::new("/tmp/pkg");
        assert_eq!(
            crate_kind_for_root(&package_root.join("examples/demo/main.rs"), package_root),
            CrateKind::Binary,
            "a nested examples/<name>/main.rs root is unambiguous and behaves like a binary",
        );
        assert_eq!(
            crate_kind_for_root(&package_root.join("tests/foo/main.rs"), package_root),
            CrateKind::Binary,
            "a nested tests/<name>/main.rs root is unambiguous and behaves like a binary",
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_crate_root_items() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::CrateRoot),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_shallow_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::ShallowPrivateModule),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_nested_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::NestedModule),
            "consider using `pub(super)` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_public_api_wording_for_libraries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Library, "struct"),
            "struct is not reachable from the crate's public API"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_subtree_wording_for_binaries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Binary, "function"),
            "function is not used outside its parent module subtree"
        );
    }
}

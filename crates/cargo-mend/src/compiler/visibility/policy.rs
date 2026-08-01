use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Result;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::annotation::VisibilityReach;
use super::scan::AllowanceReason;
use super::scan::CrateKind;
use super::scan::ModuleLocation;
use super::scan::ParentVisibility;
use super::scan::SignatureExposure;
use super::scan::SuspiciousPubAssessment;
use super::scan::SuspiciousPubInput;
use super::scan::VisibilityContext;
use super::use_sites::ParentFacadeAnalysis;
use super::use_sites::ReexportOccurrence;
use crate::compiler::constants::SOURCE_DIR_BENCHES;
use crate::compiler::constants::SOURCE_DIR_EXAMPLES;
use crate::compiler::constants::SOURCE_DIR_TESTS;
use crate::compiler::exposure;
use crate::compiler::exposure::ExposureContext;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeExportRequest;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeFixSupport;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentFacadeUsage;
use crate::reporting::FixSupport;

pub(super) fn classify_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
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

    let parent_facade_export =
        parent_facade_export_status(ctx, parent_facade_analysis, input.file_path, input.name)?;

    if let Some(assessment) = assess_parent_facade_usage(parent_facade_export.as_ref()) {
        return Ok(assessment);
    }

    if let Some(allowance) =
        assess_signature_exposure_allowance(ctx, input.def_id, input.file_path, input.name)?
    {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let stale_result = parent_facade_export.as_ref().and_then(|status| {
        let facade = status
            .use_syntax()
            .map_or_else(|| String::from("re-export"), |syntax| format!("`{syntax}`"));
        let message = match status.usage {
            ParentFacadeUsage::Unused => format!(
                "parent module also has an `unused import` warning for this {facade} at {}:{}",
                status.parent_rel_path, status.parent_line,
            ),
            ParentFacadeUsage::UsedInsideSubtreeByCratePath
            | ParentFacadeUsage::UsedInsideSubtreeByCrateImport => format!(
                "parent {facade} at {}:{} is only used through crate-relative paths inside its own subtree",
                status.parent_rel_path, status.parent_line,
            ),
            ParentFacadeUsage::UsedInsideSubtreeByRelativeImport
            | ParentFacadeUsage::UsedInsideSubtreeByRelativePath
            | ParentFacadeUsage::UsedOutsideSubtree => return None,
        };
        Some((message, status))
    });

    if matches!(input.module_location, ModuleLocation::ShallowPrivate) && stale_result.is_none() {
        return Ok(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ShallowPrivatePolicy,
        ));
    }

    let (related, fix_support, stale_parent_pub_use) = match stale_result {
        Some((message, status)) => {
            let fix_support = if status.fix_support == ParentFacadeFixSupport::Supported {
                FixSupport::PubUse
            } else {
                FixSupport::NeedsManualPubUseCleanup
            };
            (Some(message), fix_support, Some(status.clone()))
        },
        None => (None, FixSupport::None, None),
    };

    Ok(SuspiciousPubAssessment::Warn {
        fix_support,
        related,
        stale_parent_pub_use,
    })
}

// Items at depth 1 (`crate::foo`) and depth 2 (`crate::foo::bar`) both map to
// `ShallowPrivate`. Depth 2 covers the common `src/<top>/<child>.rs`
// library layout: when the top-level module is private, nothing outside its
// subtree can reach the child regardless of `pub(crate)` vs `pub(super)`, so
// the policy treats them the same as depth-1 items. Depth 3+ falls through to
// `Nested`. `pub(crate)` is still rejected at `Nested` by
// `allow_pub_crate_by_policy` alone — but the visibility scan separately
// permits it when the parent facade re-exports the item as `pub(crate) use`,
// because in that case the parent has already capped reach at `pub(crate)`
// and the source modifier should match the cap (visible-at-a-glance).
pub(super) fn resolve_module_location(tcx: TyCtxt<'_>, parent_def: LocalDefId) -> ModuleLocation {
    if parent_def == CRATE_DEF_ID {
        return ModuleLocation::CrateRoot;
    }

    let grandparent = tcx.parent_module_from_def_id(parent_def).to_local_def_id();
    if grandparent == CRATE_DEF_ID {
        return ModuleLocation::ShallowPrivate;
    }

    let great_grandparent = tcx.parent_module_from_def_id(grandparent).to_local_def_id();
    if great_grandparent == CRATE_DEF_ID {
        return ModuleLocation::ShallowPrivate;
    }

    ModuleLocation::Nested
}

pub(super) fn module_depth(tcx: TyCtxt<'_>, mut module: LocalDefId) -> usize {
    let mut depth = 0;
    while module != CRATE_DEF_ID {
        depth += 1;
        module = tcx.parent_module_from_def_id(module).into();
    }
    depth
}

pub(super) const fn allow_pub_crate_by_policy(
    crate_kind: CrateKind,
    module_location: ModuleLocation,
    parent_visibility: ParentVisibility,
) -> bool {
    match (crate_kind, module_location) {
        (CrateKind::Library, ModuleLocation::CrateRoot) => true,
        (CrateKind::IntegrationTest, _) => false,
        (_, ModuleLocation::ShallowPrivate) => {
            matches!(parent_visibility, ParentVisibility::Private)
        },
        _ => false,
    }
}

pub(super) const fn narrow_to_pub_crate_by_policy(crate_kind: CrateKind) -> bool {
    matches!(crate_kind, CrateKind::Library)
}

pub(super) fn crate_kind_for_root(root_module: &Path, package_root: &Path) -> CrateKind {
    if root_module.file_name().and_then(OsStr::to_str) == Some("lib.rs") {
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
        ModuleLocation::CrateRoot | ModuleLocation::ShallowPrivate
    ) {
        "consider using just `pub` or removing `pub(crate)` entirely"
    } else {
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    }
}

/// Suggestion for a forbidden `pub(crate)` item. Two conditions make `pub` the
/// policy's recommendation, and each names its own reason:
///
/// - **Signature exposure** — the item is structurally exposed through a reachable public signature
///   (a return type or parameter of a function reachable at `pub(crate)`). Narrowing to
///   `pub(super)` or removing the modifier fails under `private_interfaces`: the type must stay at
///   least as visible as the signature that exposes it.
/// - **A parent facade that reaches its parent module** — the parent re-exports the item one level
///   above itself, so the declaration cannot be narrowed to the parent module without failing
///   E0364. `pub(crate)` and `pub(in path)` are both forbidden by policy, which leaves `pub`; the
///   private module chain still caps the actual reach. The help repeats `pub(super) use` only when
///   that exact source spelling is known.
///
/// Otherwise defer to the location-based help.
pub(super) const fn forbidden_pub_crate_suggestion(
    module_location: ModuleLocation,
    signature_exposure: SignatureExposure,
    parent_facade_reach: Option<ParentFacadeReach>,
) -> &'static str {
    match (signature_exposure, parent_facade_reach) {
        (SignatureExposure::Present, _) => {
            "this item is exposed through a public signature; consider using `pub`"
        },
        (
            SignatureExposure::Absent,
            Some(ParentFacadeReach {
                reaches_parent: true,
                spelling: ParentFacadeSpelling::Super,
                spelling_conflict: false,
            }),
        ) => {
            "the parent module re-exports this with `pub(super) use`; consider using `pub` \
             (`pub(super)` here would not compile — the re-export would be wider than the item)"
        },
        (
            SignatureExposure::Absent,
            Some(ParentFacadeReach {
                reaches_parent: true,
                ..
            }),
        ) => {
            "the parent module re-exports this to its own parent; consider using `pub` \
             (`pub(crate)` and `pub(in ...)` are forbidden by policy)"
        },
        (SignatureExposure::Absent, _) => forbidden_pub_crate_help(module_location),
    }
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
    if !status.parent_facade_reach.spelling_conflict
        && status.parent_facade_reach.spelling == ParentFacadeSpelling::Super
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::InternalParentFacadeBoundary,
        ));
    }
    match status.usage {
        ParentFacadeUsage::UsedOutsideSubtree => Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ParentFacadeUsedOutsideParent,
        )),
        ParentFacadeUsage::UsedInsideSubtreeByRelativePath
        | ParentFacadeUsage::UsedInsideSubtreeByRelativeImport => {
            let related = Some(format!(
                "parent module uses this item as an internal facade at {}:{}",
                status.parent_rel_path, status.parent_line
            ));
            Some(SuspiciousPubAssessment::ReviewInternalParentFacade { related })
        },
        ParentFacadeUsage::UsedInsideSubtreeByCratePath
        | ParentFacadeUsage::UsedInsideSubtreeByCrateImport
        | ParentFacadeUsage::Unused => None,
    }
}

fn assess_signature_exposure_allowance(
    ctx: &VisibilityContext<'_, '_>,
    item_def_id: LocalDefId,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<Option<AllowanceReason>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    let exposure_ctx = ExposureContext {
        source_cache:   ctx.source_cache,
        settings:       ctx.settings,
        source_root:    ctx.source_root,
        tcx:            ctx.tcx,
        module_sources: ctx.module_sources,
    };
    let mut facade_exposes =
        |exposing_item_def_id: LocalDefId, child_file: &Path, exposing_item_name: &str| {
            facade_exposes_item_outside_parent(
                ctx,
                exposing_item_def_id,
                child_file,
                exposing_item_name,
            )
        };
    if exposure::child_item_is_exposed_by_other_crate_visible_signature(
        &exposure_ctx,
        item_def_id,
        file_path,
        item_name,
        &mut facade_exposes,
    )? || exposure::impl_item_is_exposed_by_exported_self_type(
        &exposure_ctx,
        item_def_id,
        item_name,
        &mut facade_exposes,
    )? || exposure::child_item_is_exposed_by_sibling_boundary_signature(
        &exposure_ctx,
        item_def_id,
        item_name,
        &mut facade_exposes,
    )? || exposure::parent_boundary_public_signature_exposes_child_used_outside_parent(
        &exposure_ctx,
        item_def_id,
        item_name,
    )? {
        return Ok(Some(AllowanceReason::ExposedByOtherCrateVisibleSignature));
    }
    Ok(None)
}

pub(super) fn parent_facade_export_status(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    child_file: &Path,
    item_name: Option<&str>,
) -> Result<Option<ParentFacadeExportStatus>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    let Some(parent_facade_analysis) = parent_facade_analysis else {
        return Ok(None);
    };
    let occurrences = &parent_facade_analysis.nearest;
    let unique_export = occurrences.matching.len() == 1;
    let selected_reach = VisibilityReach::from(occurrences.selected.visibility);
    let mut selected_status: Option<ParentFacadeExportStatus> = None;
    for occurrence in &occurrences.matching {
        if VisibilityReach::from(occurrence.visibility).compare(selected_reach, ctx.tcx)
            == Some(Ordering::Less)
        {
            continue;
        }
        let status = facade::parent_facade_export_status(ParentFacadeExportRequest {
            source_cache: ctx.source_cache,
            settings: ctx.settings,
            source_root: ctx.source_root,
            tcx: ctx.tcx,
            module_sources: ctx.module_sources,
            #[cfg(feature = "test-counters")]
            use_def_id: occurrence.use_def_id,
            owner_module: occurrence.owner_module,
            use_span: occurrence.span,
            parent_facade_reach: parent_facade_reach_for_occurrence(
                ctx,
                occurrence,
                occurrence.spelling_conflict,
            ),
            usage_by_name: &occurrence.usage_by_name,
            export_names: &occurrence.export_names,
            unique_export,
            child_file,
            item_name,
        })?;
        let Some(status) = status else {
            continue;
        };
        if selected_status.as_ref().is_none_or(|selected| {
            parent_facade_status_priority(&status) > parent_facade_status_priority(selected)
        }) {
            selected_status = Some(status);
        }
    }
    Ok(selected_status)
}

pub(super) fn parent_facade_reach_for_occurrence(
    ctx: &VisibilityContext<'_, '_>,
    occurrence: &ReexportOccurrence,
    spelling_conflict: bool,
) -> ParentFacadeReach {
    let parent_module = ctx
        .tcx
        .parent_module_from_def_id(occurrence.owner_module)
        .to_def_id();
    let parent_reach = VisibilityReach::from(Visibility::Restricted(parent_module));
    let occurrence_reach = VisibilityReach::from(occurrence.visibility);
    ParentFacadeReach {
        reaches_parent: matches!(
            occurrence_reach.compare(parent_reach, ctx.tcx),
            Some(Ordering::Equal | Ordering::Greater)
        ),
        spelling: occurrence.facade_spelling,
        spelling_conflict,
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ParentFacadeStatusPriority {
    Unused,
    CrateImport,
    CratePath,
    RelativeImport,
    RelativePath,
    UsedSuper,
    Outside,
}

const fn parent_facade_status_priority(
    status: &ParentFacadeExportStatus,
) -> ParentFacadeStatusPriority {
    if matches!(status.usage, ParentFacadeUsage::UsedOutsideSubtree) {
        return ParentFacadeStatusPriority::Outside;
    }
    if !status.parent_facade_reach.spelling_conflict
        && matches!(
            status.parent_facade_reach.spelling,
            ParentFacadeSpelling::Super
        )
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return ParentFacadeStatusPriority::UsedSuper;
    }
    match status.usage {
        ParentFacadeUsage::Unused => ParentFacadeStatusPriority::Unused,
        ParentFacadeUsage::UsedInsideSubtreeByCrateImport => {
            ParentFacadeStatusPriority::CrateImport
        },
        ParentFacadeUsage::UsedInsideSubtreeByCratePath => ParentFacadeStatusPriority::CratePath,
        ParentFacadeUsage::UsedInsideSubtreeByRelativeImport => {
            ParentFacadeStatusPriority::RelativeImport
        },
        ParentFacadeUsage::UsedInsideSubtreeByRelativePath => {
            ParentFacadeStatusPriority::RelativePath
        },
        ParentFacadeUsage::UsedOutsideSubtree => ParentFacadeStatusPriority::Outside,
    }
}

fn facade_exposes_item_outside_parent(
    ctx: &VisibilityContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let facade_subject = ctx.reexport_index.facade_subject(item_def_id);
    let parent_facade_analysis = ctx.resolve_parent_facade(item_def_id);
    Ok(parent_facade_export_status(
        ctx,
        parent_facade_analysis.as_ref(),
        child_file,
        Some(item_name),
    )?
    .is_some_and(|status| status.usage == ParentFacadeUsage::UsedOutsideSubtree)
        || ctx.reexport_index.has_public_reexport_outside_parent(
            ctx.tcx,
            item_def_id,
            facade_subject,
        ))
}

pub(super) fn has_signature_exposure_allowance(
    ctx: &VisibilityContext<'_, '_>,
    item_def_id: LocalDefId,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<bool> {
    Ok(assess_signature_exposure_allowance(ctx, item_def_id, file_path, item_name)?.is_some())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::CrateKind;
    use super::ModuleLocation;
    use super::ParentFacadeReach;
    use super::ParentFacadeSpelling;
    use super::ParentVisibility;
    use super::SignatureExposure;
    use super::allow_pub_crate_by_policy;
    use super::crate_kind_for_root;
    use super::forbidden_pub_crate_help;
    use super::forbidden_pub_crate_suggestion;
    use super::suspicious_pub_note;
    use crate::compiler::constants::SOURCE_DIR_BENCHES;
    use crate::compiler::constants::SOURCE_DIR_EXAMPLES;
    use crate::compiler::constants::SOURCE_DIR_TESTS;

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
            ModuleLocation::ShallowPrivate,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::Nested,
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
            ModuleLocation::ShallowPrivate,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::Nested,
            ParentVisibility::Private
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_integration_test_items_in_any_location() {
        for module_location in [
            ModuleLocation::CrateRoot,
            ModuleLocation::ShallowPrivate,
            ModuleLocation::Nested,
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
            forbidden_pub_crate_help(ModuleLocation::ShallowPrivate),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_nested_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::Nested),
            "consider using `pub(super)` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_suggestion_recommends_pub_when_structurally_exposed() {
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ModuleLocation::Nested,
                SignatureExposure::Present,
                None
            ),
            "this item is exposed through a public signature; consider using `pub`"
        );
    }

    #[test]
    fn forbidden_pub_crate_suggestion_states_parent_facade_syntax_only_when_known() {
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ModuleLocation::Nested,
                SignatureExposure::Absent,
                Some(ParentFacadeReach {
                    reaches_parent:    true,
                    spelling:          ParentFacadeSpelling::Super,
                    spelling_conflict: false,
                })
            ),
            "the parent module re-exports this with `pub(super) use`; consider using `pub` \
             (`pub(super)` here would not compile — the re-export would be wider than the item)"
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ModuleLocation::Nested,
                SignatureExposure::Absent,
                Some(ParentFacadeReach {
                    reaches_parent:    true,
                    spelling:          ParentFacadeSpelling::Other,
                    spelling_conflict: false,
                })
            ),
            "the parent module re-exports this to its own parent; consider using `pub` \
             (`pub(crate)` and `pub(in ...)` are forbidden by policy)"
        );
    }

    #[test]
    fn forbidden_pub_crate_suggestion_defers_to_location_help_when_not_exposed() {
        assert_eq!(
            forbidden_pub_crate_suggestion(ModuleLocation::Nested, SignatureExposure::Absent, None),
            forbidden_pub_crate_help(ModuleLocation::Nested)
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

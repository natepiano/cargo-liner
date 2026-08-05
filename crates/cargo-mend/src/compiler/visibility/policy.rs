use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Result;
use proc_macro2::Delimiter;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::annotation;
use super::annotation::ReachBoundary;
use super::annotation::VisibilityReach;
use super::annotation::VisibilitySyntax;
use super::scan::AllowanceReason;
use super::scan::CrateKind;
use super::scan::ModuleLocation;
use super::scan::ParentVisibility;
use super::scan::SuspiciousPubAssessment;
use super::scan::SuspiciousPubInput;
use super::scan::VisibilityContext;
use super::use_sites::ExactGlobSubjectResolution;
use super::use_sites::FacadeChainBlocker;
use super::use_sites::FacadeChainResolution;
use super::use_sites::ParentFacadeAnalysis;
use super::use_sites::ReexportIndex;
use super::use_sites::ReexportOccurrence;
use crate::compiler::constants::SOURCE_DIR_BENCHES;
use crate::compiler::constants::SOURCE_DIR_EXAMPLES;
use crate::compiler::constants::SOURCE_DIR_TESTS;
use crate::compiler::exposure;
use crate::compiler::exposure::BoundaryScopeCache;
use crate::compiler::exposure::ExposureContext;
use crate::compiler::exposure::ModuleScopeCache;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeExportRequest;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeFixSupport;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentFacadeUsage;
use crate::config::PubInPath;
use crate::reporting::FixSupport;

/// What a resolved parent-facade chain grants the item. `Unresolved` covers both
/// "no facade analysis" and "the chain stopped at a boundary it could not
/// resolve".
#[derive(Clone, Copy)]
enum FacadeChainReach {
    Unresolved,
    ExportedAt(VisibilityReach),
}

/// How far the item's own signature forces it to stay reachable. `Contained`
/// means no signature names it outside its own module, so the signature places
/// no floor under the declaration.
#[derive(Clone, Copy)]
pub(super) enum SignatureExposure {
    Contained,
    ExposedAt(VisibilityReach),
}

/// The narrowest reach the declaration may take once the facade chain and the
/// signature are both satisfied. `Unconstrained` means neither places a floor
/// under it.
#[derive(Clone, Copy)]
enum VisibilityRequirement {
    Unconstrained,
    AtLeast(VisibilityReach),
}

/// Whether `pub_in_path = "required"` says this declaration must be rewritten to
/// an exact `pub(in crate::…)` boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExactBoundaryNarrowing {
    Required,
    NotRequired,
}

impl From<Option<VisibilityReach>> for SignatureExposure {
    fn from(signature_exposure: Option<VisibilityReach>) -> Self {
        signature_exposure.map_or(Self::Contained, Self::ExposedAt)
    }
}

impl SignatureExposure {
    /// Whether a signature names the item from outside the crate, which no
    /// crate-internal annotation can satisfy.
    pub(super) fn reaches_outside_crate(self) -> bool {
        match self {
            Self::Contained => false,
            Self::ExposedAt(reach) => reach.is_public(),
        }
    }

    /// Whether `candidate` is at least as wide as the reach the signature
    /// forces. A `Contained` signature places no floor, so anything satisfies
    /// it.
    pub(super) fn is_satisfied_by(
        self,
        candidate: VisibilityReach,
        ctx: &VisibilityContext<'_, '_>,
    ) -> bool {
        match self {
            Self::Contained => true,
            Self::ExposedAt(required) => matches!(
                candidate.compare(required, ctx.tcx),
                Some(Ordering::Equal | Ordering::Greater)
            ),
        }
    }

    /// Widens `required` to also cover the signature's own reach.
    pub(super) fn combined_with(
        self,
        required: VisibilityReach,
        ctx: &VisibilityContext<'_, '_>,
    ) -> VisibilityReach {
        match self {
            Self::Contained => required,
            Self::ExposedAt(signature_reach) => required.join(signature_reach, ctx.tcx),
        }
    }
}

fn joined_visibility_requirement(
    tcx: TyCtxt<'_>,
    facade_chain_reach: FacadeChainReach,
    signature_exposure: SignatureExposure,
) -> VisibilityRequirement {
    match (facade_chain_reach, signature_exposure) {
        (FacadeChainReach::ExportedAt(facade), SignatureExposure::ExposedAt(signature)) => {
            VisibilityRequirement::AtLeast(facade.join(signature, tcx))
        },
        (FacadeChainReach::ExportedAt(required), SignatureExposure::Contained)
        | (FacadeChainReach::Unresolved, SignatureExposure::ExposedAt(required)) => {
            VisibilityRequirement::AtLeast(required)
        },
        (FacadeChainReach::Unresolved, SignatureExposure::Contained) => {
            VisibilityRequirement::Unconstrained
        },
    }
}

/// Answers whether this declaration has a resolved exact boundary it must be
/// narrowed to. Only a bare `pub` under `pub_in_path = "required"` qualifies, and
/// only when the facade chain resolved and the joined requirement lands on a
/// module below the crate root — `pub` and `pub(crate)` requirements are not
/// exact boundaries.
fn exact_boundary_narrowing(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    facade_chain_reach: FacadeChainReach,
    visibility_requirement: VisibilityRequirement,
) -> ExactBoundaryNarrowing {
    if !matches!(
        ctx.settings.visibility_config.pub_in_path,
        PubInPath::Required
    ) || !matches!(input.visibility_syntax, VisibilitySyntax::Public)
    {
        return ExactBoundaryNarrowing::NotRequired;
    }
    let (FacadeChainReach::ExportedAt(_), VisibilityRequirement::AtLeast(required)) =
        (facade_chain_reach, visibility_requirement)
    else {
        return ExactBoundaryNarrowing::NotRequired;
    };
    match required.boundary() {
        ReachBoundary::Module => ExactBoundaryNarrowing::Required,
        ReachBoundary::Everywhere | ReachBoundary::CrateRoot => ExactBoundaryNarrowing::NotRequired,
    }
}

pub(super) fn classify_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
) -> Result<SuspiciousPubAssessment> {
    let facade_chain_reach =
        parent_facade_analysis.map_or(FacadeChainReach::Unresolved, |analysis| {
            match analysis.chain {
                FacadeChainResolution::Resolved { required } => {
                    FacadeChainReach::ExportedAt(required)
                },
                FacadeChainResolution::Unresolvable { .. } => FacadeChainReach::Unresolved,
            }
        });
    let visibility_requirement =
        joined_visibility_requirement(ctx.tcx, facade_chain_reach, signature_exposure);
    let narrowing =
        exact_boundary_narrowing(ctx, input, facade_chain_reach, visibility_requirement);
    if narrowing == ExactBoundaryNarrowing::NotRequired
        && let Some(allowance) = basic_suspicious_pub_allowance(
            ctx,
            input.def_id,
            input.config_rel_path,
            input.parent_visibility,
            input.name,
            input.visibility_syntax,
        )
    {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let parent_facade_export =
        parent_facade_export_status(ctx, parent_facade_analysis, input.file_path, input.name)?;

    if narrowing == ExactBoundaryNarrowing::NotRequired
        && let Some(assessment) =
            assess_parent_facade_usage(parent_facade_export.as_ref(), input.visibility_syntax)
    {
        return Ok(assessment);
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

    let declared_reach = VisibilityReach::from(ctx.tcx.visibility(input.def_id.to_def_id()));
    if narrowing == ExactBoundaryNarrowing::NotRequired
        && stale_result.is_none()
        && matches!(signature_exposure, SignatureExposure::ExposedAt(_))
        && matches!(visibility_requirement, VisibilityRequirement::AtLeast(required)
            if declared_reach.compare(required, ctx.tcx) == Some(Ordering::Equal))
    {
        return Ok(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ExposedByOtherCrateVisibleSignature,
        ));
    }

    if narrowing == ExactBoundaryNarrowing::NotRequired
        && matches!(input.module_location, ModuleLocation::ShallowPrivate)
        && stale_result.is_none()
    {
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
// `ShallowPrivate`. Depth 2 covers the common `src/<top>/<child>.rs` library
// layout. Depth 3+ maps to `Nested`.
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

pub(super) enum ForbiddenPubCrateSuggestionReason {
    PublicSignatureExposure,
    NoFacadeRepair {
        boundary_path: String,
        repair:        NoFacadeVisibilityRepair,
    },
    ExactPubSuperParentFacade {
        boundary_path: String,
    },
    ExactPubSuperParentFacadeWithoutKnownBoundary,
    ParentFacade {
        boundary_path: String,
    },
    ParentFacadeWithoutKnownBoundary,
    LocationPolicy {
        module_location: ModuleLocation,
    },
}

impl ForbiddenPubCrateSuggestionReason {
    pub(super) fn headline(&self, generic_headline: String) -> String {
        match self {
            Self::NoFacadeRepair { repair, .. } => no_facade_headline(*repair, generic_headline),
            Self::PublicSignatureExposure
            | Self::ExactPubSuperParentFacade { .. }
            | Self::ExactPubSuperParentFacadeWithoutKnownBoundary
            | Self::ParentFacade { .. }
            | Self::ParentFacadeWithoutKnownBoundary
            | Self::LocationPolicy { .. } => generic_headline,
        }
    }
}

/// Suggestion for a `pub(crate)` item that can use a narrower boundary. Each
/// `ForbiddenPubCrateSuggestionReason` carries the data for exactly one of the
/// following cases:
///
/// - `PublicSignatureExposure` keeps the item at `pub` because a reachable public signature exposes
///   it.
/// - `NoFacadeRepair` carries the visibility repair and its structural boundary.
/// - The parent-facade variants distinguish exact `pub(super) use` syntax and whether the resolved
///   boundary path is known.
/// - `LocationPolicy` supplies the fallback module location.
pub(super) fn forbidden_pub_crate_suggestion(reason: ForbiddenPubCrateSuggestionReason) -> String {
    match reason {
        ForbiddenPubCrateSuggestionReason::PublicSignatureExposure => {
            String::from("this item is exposed through a public signature; consider using `pub`")
        },
        ForbiddenPubCrateSuggestionReason::NoFacadeRepair {
            boundary_path,
            repair,
        } => no_facade_suggestion(repair, &boundary_path),
        ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacade { boundary_path } => {
            format!(
                "the parent module re-exports this with `pub(super) use` to `{boundary_path}`; \
                 consider using `pub` (`pub(super)` here would not compile — the re-export would \
                 be wider than the item)"
            )
        },
        ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacadeWithoutKnownBoundary => {
            String::from(
                "the parent module re-exports this with `pub(super) use`; consider using `pub` \
                 (`pub(super)` here would not compile — the re-export would be wider than the \
                 item)",
            )
        },
        ForbiddenPubCrateSuggestionReason::ParentFacade { boundary_path } => format!(
            "the parent module re-exports this to `{boundary_path}`; use the narrowest source \
             visibility that still reaches that boundary"
        ),
        ForbiddenPubCrateSuggestionReason::ParentFacadeWithoutKnownBoundary => String::from(
            "the parent module re-exports this to its own parent; resolve that boundary before \
             narrowing the source visibility",
        ),
        ForbiddenPubCrateSuggestionReason::LocationPolicy { module_location } => {
            forbidden_pub_crate_help(module_location).to_string()
        },
    }
}

#[derive(Clone, Copy)]
pub(in crate::compiler) enum NoFacadeVisibilityRepair {
    RemoveAnnotation,
    UseParentVisibility,
    StructuralMigrationForCallerLocations,
    StructuralMigrationForSignatureReach { required: VisibilityReach },
}

impl NoFacadeVisibilityRepair {
    const REMOVE_ANNOTATION_SUGGESTION: &str = "consider removing the visibility";
    const STRUCTURAL_HEADLINE: &str = "no policy-allowed visibility keeps this item reachable where it is used: private and \
         `pub(super)` are too narrow, and no facade caps `pub`";

    fn suggestion(self, boundary_path: &str) -> String {
        match self {
            Self::RemoveAnnotation => String::from(Self::REMOVE_ANNOTATION_SUGGESTION),
            Self::UseParentVisibility => consider_using("pub(super)"),
            Self::StructuralMigrationForCallerLocations
            | Self::StructuralMigrationForSignatureReach { .. } => format!(
                "move the item into `{boundary_path}`, or add an explicit facade at \
                 `{boundary_path}` and rerun `cargo mend`"
            ),
        }
    }

    fn headline(self, generic_headline: String) -> String {
        match self {
            Self::RemoveAnnotation | Self::UseParentVisibility => generic_headline,
            Self::StructuralMigrationForCallerLocations
            | Self::StructuralMigrationForSignatureReach { .. } => {
                String::from(Self::STRUCTURAL_HEADLINE)
            },
        }
    }

    /// Keeps whichever repair demands more of the caller. A declaration reached
    /// from several compilation targets must satisfy every one of them, so the
    /// group's repair is the most invasive its constraints require.
    ///
    /// Ties keep `self`, so the first repair encountered wins among equals.
    pub(in crate::compiler) const fn most_invasive(self, other: Self) -> Self {
        if other.invasiveness() > self.invasiveness() {
            other
        } else {
            self
        }
    }

    const fn invasiveness(self) -> u8 {
        match self {
            Self::RemoveAnnotation => 0,
            Self::UseParentVisibility => 1,
            Self::StructuralMigrationForCallerLocations
            | Self::StructuralMigrationForSignatureReach { .. } => 2,
        }
    }
}

/// The one phrasing for a suggestion that names a replacement visibility.
///
/// Every diagnostic proposing a different annotation routes through here, so the
/// wording stays identical across all of them.
pub(super) fn consider_using(visibility: &str) -> String {
    format!("consider using: `{visibility}`")
}

pub(in crate::compiler) fn no_facade_suggestion(
    repair: NoFacadeVisibilityRepair,
    boundary_path: &str,
) -> String {
    repair.suggestion(boundary_path)
}

pub(in crate::compiler) fn no_facade_headline(
    repair: NoFacadeVisibilityRepair,
    generic_headline: String,
) -> String {
    repair.headline(generic_headline)
}

pub(in crate::compiler) fn classify_no_facade_callers(
    item_module: &str,
    parent_scope: &str,
    callers: &BTreeSet<String>,
) -> NoFacadeVisibilityRepair {
    // An empty scope has no callers outside it, so it never rules a repair out.
    // `parent_scope` is empty for a crate-root item, whose callers all satisfy
    // the `item_module` test above first.
    if item_module.is_empty()
        || callers
            .iter()
            .all(|caller| def_path_is_descendant(caller, item_module))
    {
        return NoFacadeVisibilityRepair::RemoveAnnotation;
    }
    if parent_scope.is_empty()
        || callers
            .iter()
            .all(|caller| def_path_is_descendant(caller, parent_scope))
    {
        return NoFacadeVisibilityRepair::UseParentVisibility;
    }
    NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
}

/// Returns the deepest module containing both `item_module` and every caller.
/// `None` represents the crate root.
pub(in crate::compiler) fn common_ancestor_def_path(
    item_module: &str,
    callers: &BTreeSet<String>,
) -> Option<String> {
    let mut boundary = item_module;
    loop {
        if boundary.is_empty() || boundary == "crate" {
            return None;
        }
        if callers
            .iter()
            .all(|caller| def_path_is_descendant(caller, boundary))
        {
            return Some(boundary.to_string());
        }
        boundary = boundary.rsplit_once("::").map(|(parent, _)| parent)?;
    }
}

pub(in crate::compiler) fn crate_rooted_def_path(def_path: &str) -> String {
    match def_path {
        "" | "crate" => String::from("crate"),
        rooted if rooted.starts_with("crate::") => rooted.to_string(),
        relative => format!("crate::{relative}"),
    }
}

pub(in crate::compiler) fn parent_scope_def_path(item_module: &str) -> &str {
    item_module
        .rsplit_once("::")
        .map_or("", |(parent, _)| parent)
}

pub(super) fn canonical_pub_in_boundary(item_module: &str, source: &str) -> Option<String> {
    let tokens = source.parse::<TokenStream>().ok()?;
    let mut tokens = tokens.into_iter();
    let group = loop {
        let token = tokens.next()?;
        if matches!(&token, TokenTree::Ident(ident) if ident == "pub") {
            let TokenTree::Group(group) = tokens.next()? else {
                return None;
            };
            break group;
        }
    };
    if group.delimiter() != Delimiter::Parenthesis {
        return None;
    }
    let mut restricted = group.stream().into_iter();
    if !matches!(restricted.next(), Some(TokenTree::Ident(ident)) if ident == "in") {
        return None;
    }
    let mut path = restricted.collect::<TokenStream>().to_string();
    path.retain(|character| !character.is_whitespace());
    if path == "crate" || path.starts_with("crate::") {
        return Some(path);
    }

    let mut module_segments = item_module.split("::").collect::<Vec<_>>();
    let mut path_segments = path.split("::");
    match path_segments.next()? {
        "self" => {},
        "super" => {
            module_segments.pop()?;
            for segment in path_segments.by_ref() {
                if segment != "super" {
                    return None;
                }
                module_segments.pop()?;
            }
        },
        _ => return None,
    }
    let module_path = module_segments.join("::");
    if module_path.is_empty() {
        Some(String::from("crate"))
    } else {
        Some(format!("crate::{module_path}"))
    }
}

/// Whether `path` names `ancestor` itself or something nested inside it.
///
/// An empty `ancestor` is not treated as a universal ancestor — callers that
/// want that must say so; see `classify_no_facade_callers`.
pub(in crate::compiler) fn def_path_is_descendant(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|remainder| remainder.starts_with("::"))
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
    visibility_syntax: VisibilitySyntax,
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
    if !matches!(visibility_syntax, VisibilitySyntax::Public) {
        return None;
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
    visibility_syntax: VisibilitySyntax,
) -> Option<SuspiciousPubAssessment> {
    let status = parent_facade_export?;
    if !matches!(visibility_syntax, VisibilitySyntax::Public)
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::InternalParentFacadeBoundary,
        ));
    }
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
) -> Result<Option<VisibilityReach>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    let exposure_ctx = ExposureContext {
        source_cache:             ctx.source_cache,
        settings:                 ctx.settings,
        source_root:              ctx.source_root,
        tcx:                      ctx.tcx,
        module_sources:           ctx.module_sources,
        reexport_index:           ctx.reexport_index,
        module_scope_cache:       ModuleScopeCache::default(),
        boundary_scope_cache:     BoundaryScopeCache::default(),
        signature_exposure_cache: &ctx.signature_exposure_cache,
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
    let child_signature_exposure =
        exposure::child_item_is_exposed_by_other_crate_visible_signature(
            &exposure_ctx,
            item_def_id,
            file_path,
            item_name,
            &mut facade_exposes,
        )?;
    let impl_signature_exposure = exposure::impl_item_is_exposed_by_exported_self_type(
        &exposure_ctx,
        item_def_id,
        item_name,
        &mut facade_exposes,
    )?;
    let sibling_signature_exposure = exposure::child_item_is_exposed_by_sibling_boundary_signature(
        &exposure_ctx,
        item_def_id,
        item_name,
        &mut facade_exposes,
    )?;
    let parent_signature_exposure =
        exposure::parent_boundary_public_signature_exposes_child_used_outside_parent(
            &exposure_ctx,
            item_def_id,
            item_name,
            &mut facade_exposes,
        )?;

    let mut signature_exposure: Option<VisibilityReach> = None;
    for exposure_reach in [
        child_signature_exposure,
        impl_signature_exposure,
        sibling_signature_exposure,
        parent_signature_exposure,
    ] {
        let Some(exposure_reach) = exposure_reach else {
            continue;
        };
        signature_exposure = Some(signature_exposure.map_or(exposure_reach, |current| {
            current.join(exposure_reach, ctx.tcx)
        }));
    }
    Ok(signature_exposure)
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
) -> Result<Option<VisibilityReach>> {
    let facade_subject = ctx.reexport_index.facade_subject(item_def_id);
    let parent_facade_analysis = ctx.resolve_parent_facade(item_def_id);
    let facade_reach = parent_facade_analysis
        .as_ref()
        .and_then(|analysis| match analysis.chain {
            FacadeChainResolution::Resolved { required } => Some(required),
            FacadeChainResolution::Unresolvable { blocker } => {
                let (occurrence, effective_visibility) = match blocker {
                    FacadeChainBlocker::ForeignBoundary(occurrence) => {
                        (occurrence, occurrence.visibility)
                    },
                    FacadeChainBlocker::Glob(occurrence) => {
                        let ExactGlobSubjectResolution::Resolved { visibility } =
                            ReexportIndex::exact_glob_subject_resolution(
                                ctx.tcx,
                                facade_subject.to_def_id(),
                                occurrence,
                            )
                        else {
                            return None;
                        };
                        (occurrence, visibility)
                    },
                };
                annotation::capped_by_enclosing_modules(
                    VisibilityReach::from(effective_visibility),
                    occurrence.use_def_id,
                    ctx.tcx,
                )
            },
        });
    let mut outward_reach = parent_facade_export_status(
        ctx,
        parent_facade_analysis.as_ref(),
        child_file,
        Some(item_name),
    )?
    .filter(|status| status.usage == ParentFacadeUsage::UsedOutsideSubtree)
    .and(facade_reach);
    for reach in ctx
        .reexport_index
        .applicable_reexport_reaches_outside_parent(ctx.tcx, item_def_id, facade_subject)
    {
        outward_reach = Some(outward_reach.map_or(reach, |current: VisibilityReach| {
            current.join(reach, ctx.tcx)
        }));
    }
    for reach in ctx
        .reexport_index
        .applicable_exported_ancestor_path_reaches(ctx.tcx, item_def_id)
    {
        outward_reach = Some(outward_reach.map_or(reach, |current: VisibilityReach| {
            current.join(reach, ctx.tcx)
        }));
    }
    Ok(outward_reach)
}

pub(super) fn signature_exposure_reach(
    ctx: &VisibilityContext<'_, '_>,
    item_def_id: LocalDefId,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<SignatureExposure> {
    assess_signature_exposure_allowance(ctx, item_def_id, file_path, item_name)
        .map(SignatureExposure::from)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::CrateKind;
    use super::ForbiddenPubCrateSuggestionReason;
    use super::ModuleLocation;
    use super::NoFacadeVisibilityRepair;
    use super::classify_no_facade_callers;
    use super::common_ancestor_def_path;
    use super::crate_kind_for_root;
    use super::crate_rooted_def_path;
    use super::forbidden_pub_crate_help;
    use super::forbidden_pub_crate_suggestion;
    use super::suspicious_pub_note;
    use crate::compiler::constants::SOURCE_DIR_BENCHES;
    use crate::compiler::constants::SOURCE_DIR_EXAMPLES;
    use crate::compiler::constants::SOURCE_DIR_TESTS;

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
    fn forbidden_pub_crate_suggestion_renders_signature_and_location_reasons() {
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ForbiddenPubCrateSuggestionReason::PublicSignatureExposure,
            ),
            "this item is exposed through a public signature; consider using `pub`",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(ForbiddenPubCrateSuggestionReason::LocationPolicy {
                module_location: ModuleLocation::Nested,
            },),
            forbidden_pub_crate_help(ModuleLocation::Nested),
        );
    }

    #[test]
    fn forbidden_pub_crate_suggestion_renders_no_facade_reasons() {
        assert_eq!(
            forbidden_pub_crate_suggestion(ForbiddenPubCrateSuggestionReason::NoFacadeRepair {
                boundary_path: String::from("crate::a"),
                repair:        NoFacadeVisibilityRepair::RemoveAnnotation,
            },),
            "consider removing the visibility",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(ForbiddenPubCrateSuggestionReason::NoFacadeRepair {
                boundary_path: String::from("crate::a"),
                repair:        NoFacadeVisibilityRepair::UseParentVisibility,
            },),
            "consider using: `pub(super)`",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(ForbiddenPubCrateSuggestionReason::NoFacadeRepair {
                boundary_path: String::from("crate::a"),
                repair:        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations,
            },),
            "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun \
             `cargo mend`",
        );
    }

    #[test]
    fn forbidden_pub_crate_suggestion_renders_parent_facade_reasons() {
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacade {
                    boundary_path: String::from("crate::a"),
                },
            ),
            "the parent module re-exports this with `pub(super) use` to `crate::a`; consider using `pub` \
             (`pub(super)` here would not compile — the re-export would be wider than the item)",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacadeWithoutKnownBoundary,
            ),
            "the parent module re-exports this with `pub(super) use`; consider using `pub` \
             (`pub(super)` here would not compile — the re-export would be wider than the item)",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(ForbiddenPubCrateSuggestionReason::ParentFacade {
                boundary_path: String::from("crate::a"),
            }),
            "the parent module re-exports this to `crate::a`; use the narrowest source \
             visibility that still reaches that boundary",
        );
        assert_eq!(
            forbidden_pub_crate_suggestion(
                ForbiddenPubCrateSuggestionReason::ParentFacadeWithoutKnownBoundary,
            ),
            "the parent module re-exports this to its own parent; resolve that boundary before \
             narrowing the source visibility",
        );
    }

    #[test]
    fn no_facade_visibility_repair_widens_monotonically_with_added_callers() {
        let item_module = "a::b::c";
        let parent_scope = "a::b";
        let callers = BTreeSet::from([String::from("a::b::c")]);
        let repair = classify_no_facade_callers(item_module, parent_scope, &callers);
        assert!(matches!(repair, NoFacadeVisibilityRepair::RemoveAnnotation));

        let callers = BTreeSet::from([String::from("a::b::c"), String::from("a::b::sibling")]);
        let repair = classify_no_facade_callers(item_module, parent_scope, &callers);
        assert!(matches!(
            repair,
            NoFacadeVisibilityRepair::UseParentVisibility
        ));

        let callers = BTreeSet::from([String::from("a::b::c"), String::from("a::caller")]);
        let repair = classify_no_facade_callers(item_module, parent_scope, &callers);
        assert!(match repair {
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => true,
            NoFacadeVisibilityRepair::RemoveAnnotation
            | NoFacadeVisibilityRepair::UseParentVisibility
            | NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { .. } => false,
        });
        assert_eq!(
            repair.suggestion("crate::a"),
            "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun \
             `cargo mend`",
        );
    }

    #[test]
    fn common_ancestor_uses_complete_path_segments() {
        let callers = BTreeSet::from([
            String::from("root::panel::diegetic"),
            String::from("root::panel::preview::camera"),
        ]);
        assert_eq!(
            common_ancestor_def_path("root::panel::conversion::saved", &callers),
            Some(String::from("root::panel")),
        );

        let callers = BTreeSet::from([String::from("root::panel_views")]);
        assert_eq!(
            common_ancestor_def_path("root::panel::conversion", &callers),
            Some(String::from("root")),
        );

        let callers = BTreeSet::from([String::from("other")]);
        assert_eq!(common_ancestor_def_path("root::panel", &callers), None);
    }

    #[test]
    fn empty_def_path_resolves_to_the_crate_root() {
        assert_eq!(crate_rooted_def_path(""), "crate");
    }

    #[test]
    fn relative_pub_in_paths_resolve_against_the_item_module() {
        assert_eq!(
            super::canonical_pub_in_boundary(
                "a::b::c",
                "pub /* policy */ (in super :: super) fn helper() {}",
            ),
            Some(String::from("crate::a")),
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

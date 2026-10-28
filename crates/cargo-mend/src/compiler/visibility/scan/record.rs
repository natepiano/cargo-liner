use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ffi::OsStr;

use anyhow::Result;
use rustc_hir::def::CtorKind;
use rustc_hir::def::DefKind;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::Symbol;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::ExposureConsumers;
use super::FindingParams;
use super::ItemCategory;
use super::ItemInfo;
use super::SuspiciousPubAssessment;
use super::SuspiciousPubInput;
use super::VisibilityContext;
use super::classify;
use super::classify::CrateKind;
use super::classify::ModuleLocation;
use super::classify::ParentVisibility;
use super::classify::VisibilityFindingContext;
use crate::compiler::cfg_excluded_references::CfgExcludedReference;
use crate::compiler::constants::PRELUDE_MODULE_NAME;
use crate::compiler::facade;
use crate::compiler::facade::ExternalChildUsage;
use crate::compiler::facade::FacadeSpellingAgreement;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeFixSupport;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentReach;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredCallerReconciliation;
use crate::compiler::persistence::StoredConstraintOutcome;
use crate::compiler::persistence::StoredExactBoundaryAcceptance;
use crate::compiler::persistence::StoredExactPathPolicy;
use crate::compiler::persistence::StoredFacadeConstraint;
use crate::compiler::persistence::StoredFinding;
use crate::compiler::persistence::StoredInterfaceCeiling;
use crate::compiler::persistence::StoredPubUseFixFact;
use crate::compiler::persistence::StoredVisibilityConstraint;
use crate::compiler::persistence::StoredVisibilityDeclaration;
use crate::compiler::persistence::StoredVisibilityReach;
use crate::compiler::persistence::StoredVisibilitySpelling;
use crate::compiler::visibility::annotation::PathSpelling;
use crate::compiler::visibility::annotation::VisibilityAnnotation;
use crate::compiler::visibility::annotation::VisibilityReach;
use crate::compiler::visibility::annotation::VisibilitySyntax;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::policy::NoFacadeVisibilityRepair;
use crate::compiler::visibility::policy::OverbroadPubCrateSuggestionReason;
use crate::compiler::visibility::policy::SignatureExposure;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::compiler::visibility::use_sites::FacadeChainBlocker;
use crate::compiler::visibility::use_sites::FacadeChainResolution;
use crate::compiler::visibility::use_sites::ParentFacadeAnalysis;
use crate::compiler::visibility::use_sites::RetainedFacadeRequirement;
use crate::config::DiagnosticCode;
use crate::config::PreludePubMod;
use crate::config::PubInPath;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

enum PubCrateNarrowing {
    Ineligible,
    Shallow,
    Nested,
}

impl PubCrateNarrowing {
    const fn classify(
        syntax: VisibilitySyntax,
        parent_visibility: ParentVisibility,
        logical_module_depth: usize,
    ) -> Self {
        match (syntax, parent_visibility, logical_module_depth) {
            (VisibilitySyntax::Public, ParentVisibility::Private, 1) => Self::Shallow,
            (VisibilitySyntax::Public, ParentVisibility::Private, 2..) => Self::Nested,
            _ => Self::Ineligible,
        }
    }
}

pub(super) fn record_visibility_findings(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(annotation) =
        VisibilityAnnotation::from_item(item.visibility_text, item.def_id, ctx.tcx)
    else {
        return Ok(());
    };
    let finding_context = classify::visibility_finding_context(ctx, item);
    let parent_facade_analysis = ctx.resolve_parent_facade(item.def_id);
    let signature_exposure = super::signature_exposure_for_annotation(
        ctx,
        item,
        &annotation,
        ExposureConsumers::AllVisibilityFindings,
    )?;

    if record_forbidden_visibility_annotation(
        ctx,
        item,
        &annotation,
        &finding_context,
        parent_facade_analysis.as_ref(),
        signature_exposure,
        sink,
    )? {
        return Ok(());
    }
    record_review_pub_mod(ctx, item, &annotation, &finding_context, sink)?;
    maybe_record_unused_pub(
        ctx,
        item,
        &annotation,
        &finding_context,
        parent_facade_analysis.as_ref(),
        signature_exposure,
        sink,
    )?;

    match PubCrateNarrowing::classify(
        annotation.syntax(),
        finding_context.parent_visibility,
        finding_context.logical_module_depth,
    ) {
        PubCrateNarrowing::Ineligible => {},
        PubCrateNarrowing::Shallow => {
            maybe_record_narrow_to_pub_crate(ctx, item, signature_exposure, sink)?;
        },
        PubCrateNarrowing::Nested => {
            maybe_record_narrow_to_pub_crate_nested(
                ctx,
                item,
                parent_facade_analysis.as_ref(),
                signature_exposure,
                sink,
            )?;
        },
    }

    if matches!(
        annotation.syntax(),
        VisibilitySyntax::Public | VisibilitySyntax::InPath(PathSpelling::CrateRooted)
    ) && finding_context.logical_module_depth > 1
    {
        maybe_record_suspicious_pub(
            ctx,
            &SuspiciousPubInput {
                def_id:            item.def_id,
                file_path:         item.file_path,
                config_rel_path:   finding_context.config_rel_path.as_deref(),
                parent_visibility: finding_context.parent_visibility,
                module_location:   finding_context.module_location,
                crate_kind:        finding_context.crate_kind,
                kind_label:        item.kind_label,
                name:              item.name,
                highlight_span:    item.highlight_span,
                visibility_syntax: annotation.syntax(),
                category:          item.category,
                facade_subject:    item.facade_subject,
            },
            &annotation,
            parent_facade_analysis.as_ref(),
            signature_exposure,
            sink,
        )?;
    }
    Ok(())
}

fn maybe_record_unused_pub(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<()> {
    if !matches!(annotation.syntax(), VisibilitySyntax::Public)
        || item.category == ItemCategory::Module
    {
        return Ok(());
    }
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    if finding_context.crate_kind == CrateKind::Library
        && finding_context.module_location == ModuleLocation::CrateRoot
    {
        return Ok(());
    }
    if finding_context.parent_visibility == ParentVisibility::Public {
        return Ok(());
    }
    if pub_item_is_allowlisted(ctx, finding_context.config_rel_path.as_deref(), name) {
        return Ok(());
    }
    if ctx
        .effective_visibilities
        .is_public_at_level(item.def_id, Level::Reachable)
    {
        return Ok(());
    }
    // `SignatureExposure::Contained` does not preserve `pub` in
    // `maybe_record_unused_pub`: nothing names the item from outside, so an
    // unreachable annotation is exactly what this diagnostic reports.
    let annotation_reaches_signature =
        matches!(signature_exposure, SignatureExposure::ExposedAt(_))
            && signature_exposure.is_satisfied_by(annotation.reach(item.def_id, ctx.tcx), ctx);
    if parent_facade_exports_item(parent_facade_analysis)
        || facade::external_child_usage(
            ctx.source_cache,
            ctx.source_root,
            ctx.tcx,
            ctx.module_sources,
            &use_sites::parent_module_path_segments(ctx.tcx, item.def_id),
            name,
        ) == ExternalChildUsage::Present
        || annotation_reaches_signature
    {
        return Ok(());
    }

    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            diagnostic_code:         DiagnosticCode::UnusedPub,
            item:                    Some(StoredFinding::render_item(kind_label, name)),
            message:                 format!(
                "{kind_label} is not used outside its defining module"
            ),
            suggestion:              Some(String::from("consider removing `pub`")),
            fix_support:             FixSupport::UnusedPub,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           Some(use_sites::def_path_string(ctx.tcx, item.def_id)),
            narrower_scope_def_path: Some(use_sites::parent_module_def_path(ctx.tcx, item.def_id)),
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?);
    Ok(())
}

fn pub_item_is_allowlisted(
    ctx: &VisibilityContext<'_, '_>,
    config_rel_path: Option<&str>,
    item_name: &str,
) -> bool {
    let Some(path) = config_rel_path else {
        return false;
    };
    let item_key = format!("{path}::{item_name}");
    ctx.settings
        .visibility_config
        .allow_pub_items
        .iter()
        .any(|allowed| allowed == &item_key)
}

pub(super) fn record_forbidden_visibility_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<bool> {
    if matches!(annotation.syntax(), VisibilitySyntax::Crate)
        && facade_is_possible(ctx, item.def_id)
        && joined_required_reach(
            ctx,
            resolved_facade_reach(parent_facade_analysis),
            signature_exposure,
        )
        .is_some_and(|required| reach_is_pub_crate(required, ctx))
    {
        record_visibility_constraint(
            ctx,
            item,
            annotation,
            VisibilityConstraintInput {
                diagnostic_code: DiagnosticCode::OverbroadPubCrate,
                parent_facade_analysis,
                signature_exposure,
                outcome: StoredConstraintOutcome::Accepted,
            },
            sink,
        );
        return Ok(false);
    }
    match annotation.syntax() {
        // `true` means `annotation` is handled, not that a finding was recorded.
        // `record_overbroad_pub_crate` and `record_forbidden_pub_in_crate`
        // return `true` on every path reaching a decision, and both callers
        // stop there. Returning `false` would pass a `pub(crate) mod` to
        // `record_review_pub_mod`, which reports every module it receives:
        // suppressing one diagnostic would produce another on the same line.
        //
        // Both narrowing diagnostics need `cfg_excluded_source_names_item`:
        // `DiagnosticCode::OverbroadPubCrate` suggests a narrower spelling,
        // and `DiagnosticCode::ForbiddenPubInCrate` suggests removing the
        // annotation outright.
        // Either repair would prevent the excluded configuration compiling.
        VisibilitySyntax::Crate
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InParent
        | VisibilitySyntax::InCurrent
        | VisibilitySyntax::InPath(_)
            if cfg_excluded_source_names_item(ctx, item) =>
        {
            Ok(true)
        },
        VisibilitySyntax::Crate | VisibilitySyntax::InCrate => record_overbroad_pub_crate(
            ctx,
            item,
            annotation,
            finding_context,
            parent_facade_analysis,
            signature_exposure,
            sink,
        ),
        VisibilitySyntax::InParent | VisibilitySyntax::InCurrent | VisibilitySyntax::InPath(_) => {
            record_forbidden_pub_in_crate(
                ctx,
                item,
                annotation,
                finding_context,
                parent_facade_analysis,
                signature_exposure,
                sink,
            )
        },
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current => Ok(false),
    }
}

fn resolved_facade_reach(
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> Option<VisibilityReach> {
    let FacadeChainResolution::Resolved { required } = parent_facade_analysis?.chain else {
        return None;
    };
    Some(required)
}

#[derive(Clone, Copy)]
struct VisibilityConstraintInput<'analysis, 'facade> {
    diagnostic_code:        DiagnosticCode,
    parent_facade_analysis: Option<&'analysis ParentFacadeAnalysis<'facade>>,
    signature_exposure:     SignatureExposure,
    outcome:                StoredConstraintOutcome,
}

fn record_visibility_constraint(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    input: VisibilityConstraintInput<'_, '_>,
    sink: &mut FindingsSink,
) {
    let VisibilityConstraintInput {
        diagnostic_code,
        parent_facade_analysis,
        signature_exposure,
        outcome,
    } = input;
    let Some(declared_reach) = stored_visibility_reach(annotation.reach(item.def_id, ctx.tcx), ctx)
    else {
        return;
    };
    let signature_requirement = match signature_exposure {
        SignatureExposure::Contained => None,
        SignatureExposure::ExposedAt(reach) => stored_visibility_reach(reach, ctx),
    };
    let interface_ceiling = ctx
        .interface_ceilings
        .get(&item.def_id)
        .and_then(|ceiling| {
            Some(StoredInterfaceCeiling {
                reach:       stored_visibility_reach(ceiling.reach, ctx)?,
                leaked_type: ceiling.leaked_type.clone(),
                impl_header: ceiling.impl_header.clone(),
            })
        });
    let facade = match parent_facade_analysis.map(|analysis| analysis.chain) {
        Some(FacadeChainResolution::Resolved { required }) => {
            let Some(required) = stored_visibility_reach(required, ctx) else {
                return;
            };
            StoredFacadeConstraint::Resolved { required }
        },
        Some(FacadeChainResolution::Unresolvable { .. }) => StoredFacadeConstraint::Blocked,
        None if facade_less_annotation_repair_is_supported(ctx, item.def_id) => {
            StoredFacadeConstraint::Impossible
        },
        None => StoredFacadeConstraint::Absent,
    };
    let exact_boundary_acceptance =
        exact_boundary_acceptance(ctx, item, annotation, diagnostic_code);
    let annotation_edit_acceptance = annotation_edit_acceptance(item, annotation, diagnostic_code);
    let pub_in_reconciles_callers = diagnostic_code == DiagnosticCode::ForbiddenPubInCrate
        && matches!(annotation.syntax(), VisibilitySyntax::InPath(_));
    let pub_crate_reconciles_callers = diagnostic_code == DiagnosticCode::OverbroadPubCrate
        && matches!(
            annotation.syntax(),
            VisibilitySyntax::Crate | VisibilitySyntax::InCrate
        );
    let suspicious_pub_reconciles_callers = diagnostic_code == DiagnosticCode::SuspiciousPub
        && matches!(annotation.syntax(), VisibilitySyntax::Public);
    let caller_reconciliation = if item.category != ItemCategory::Use
        && parent_facade_analysis.is_none()
        && (pub_in_reconciles_callers
            || pub_crate_reconciles_callers
            || suspicious_pub_reconciles_callers)
    {
        StoredCallerReconciliation::CallerAware
    } else {
        StoredCallerReconciliation::Fixed
    };
    sink.visibility_constraints
        .push(StoredVisibilityConstraint {
            diagnostic_code,
            source: source::stored_visibility_source(ctx.tcx, item.file_path, item.highlight_span),
            declaration: StoredVisibilityDeclaration {
                item_def_path:        use_sites::def_path_string(ctx.tcx, item.def_id),
                item_module_def_path: use_sites::parent_module_def_path(ctx.tcx, item.def_id),
            },
            visibility_annotation: annotation.source().to_string(),
            declared_reach,
            spelling: stored_visibility_spelling(annotation.syntax()),
            signature_requirement,
            interface_ceiling,
            facade,
            exact_boundary_acceptance,
            annotation_edit_acceptance,
            exact_path_policy: match ctx.settings.visibility_config.pub_in_path {
                PubInPath::Forbidden => StoredExactPathPolicy::Forbidden,
                PubInPath::Permitted | PubInPath::Required => StoredExactPathPolicy::Allowed,
            },
            caller_reconciliation,
            outcome,
        });
}

/// Whether the item's declaration form lets mend edit its visibility
/// annotation at all.
///
/// `forbidden_pub_in_crate` accepts a `use` re-export here as well as a
/// declaration. It once took declarations only, which left a re-export whose
/// boundary mend had already resolved reported on every run and repaired on
/// none.
///
/// [`exact_boundary_acceptance`] answers the neighboring question — whether the
/// boundary the annotation already names is acceptable as written — and folds
/// `pub_in_path` into its answer. That setting governs what mend may *write*, so
/// it bears on a replacement that spells an exact path and not on one that
/// removes the annotation. Keeping the two apart is what lets a bounded
/// `pub(in crate::a)` have its annotation removed under
/// `pub_in_path = "forbidden"` — the only setting under which
/// `forbidden_pub_in_crate` fires at all.
const fn annotation_edit_acceptance(
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    diagnostic_code: DiagnosticCode,
) -> StoredExactBoundaryAcceptance {
    let eligible = match diagnostic_code {
        DiagnosticCode::OverbroadPubCrate => {
            matches!(
                annotation.syntax(),
                VisibilitySyntax::Crate | VisibilitySyntax::InCrate
            )
        },
        DiagnosticCode::ForbiddenPubInCrate => {
            // An `ItemCategory::Use` re-export has an annotation like a declaration.
            // `UseSiteCollector::record_qpath` records the target definition for
            // every mention regardless of its path, so references through a
            // re-export and directly to its declaration enter the same caller map.
            // The resolved boundary is therefore a conservative floor for a
            // re-export rather than a narrower boundary.
            //
            // Exclude `ItemCategory::Module`: a `mod` visibility governs an entire
            // subtree and needs a review decision rather than a rewrite.
            matches!(item.category, ItemCategory::Declaration | ItemCategory::Use)
                && matches!(
                    annotation.syntax(),
                    VisibilitySyntax::InPath(PathSpelling::CrateRooted)
                )
        },
        DiagnosticCode::SuspiciousPub => matches!(annotation.syntax(), VisibilitySyntax::Public),
        DiagnosticCode::ReviewPubMod
        | DiagnosticCode::UnusedPub
        | DiagnosticCode::PreferModuleImport
        | DiagnosticCode::InlinePathQualifiedType
        | DiagnosticCode::ShortenLocalCrateImport
        | DiagnosticCode::ReplaceDeepSuperImport
        | DiagnosticCode::WildcardParentPubUse
        | DiagnosticCode::InternalParentPubUseFacade
        | DiagnosticCode::NarrowToPubCrate
        | DiagnosticCode::FieldVisibilityWiderThanType
        | DiagnosticCode::ImportsAtTop
        | DiagnosticCode::PubUseOutsideSubtree => false,
    };
    if eligible {
        StoredExactBoundaryAcceptance::Eligible
    } else {
        StoredExactBoundaryAcceptance::Ineligible
    }
}

/// Whether the boundary the annotation already names is acceptable as written,
/// which every suppression path in `persistence::visibility_constraint` asks
/// before dropping a finding.
///
/// This is [`annotation_edit_acceptance`] plus the `pub_in_path` setting. Only
/// `forbidden_pub_in_crate` consults that setting: it is the diagnostic whose
/// annotation names an exact path, and the other two never do.
const fn exact_boundary_acceptance(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    diagnostic_code: DiagnosticCode,
) -> StoredExactBoundaryAcceptance {
    let policy_permits_the_spelling =
        !matches!(diagnostic_code, DiagnosticCode::ForbiddenPubInCrate)
            || matches!(
                ctx.settings.visibility_config.pub_in_path,
                PubInPath::Permitted | PubInPath::Required
            );
    if policy_permits_the_spelling {
        annotation_edit_acceptance(item, annotation, diagnostic_code)
    } else {
        StoredExactBoundaryAcceptance::Ineligible
    }
}

const fn stored_visibility_spelling(
    visibility_syntax: VisibilitySyntax,
) -> StoredVisibilitySpelling {
    match visibility_syntax {
        VisibilitySyntax::Public => StoredVisibilitySpelling::Public,
        VisibilitySyntax::Crate => StoredVisibilitySpelling::Crate,
        VisibilitySyntax::InCrate => StoredVisibilitySpelling::InCrate,
        VisibilitySyntax::InPath(PathSpelling::CrateRooted) => StoredVisibilitySpelling::ExactPath,
        VisibilitySyntax::Private
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InParent
        | VisibilitySyntax::InCurrent
        | VisibilitySyntax::InPath(PathSpelling::Relative) => {
            StoredVisibilitySpelling::NonCanonical
        },
    }
}

fn stored_visibility_reach(
    reach: VisibilityReach,
    ctx: &VisibilityContext<'_, '_>,
) -> Option<StoredVisibilityReach> {
    let boundary = visibility_reach_boundary_path(reach, ctx)?;
    Some(match boundary.as_str() {
        "crate-external" => StoredVisibilityReach::Public,
        "crate" => StoredVisibilityReach::Crate,
        _ => StoredVisibilityReach::Restricted { boundary },
    })
}

fn record_overbroad_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<bool> {
    if item.category == ItemCategory::Use && matches!(annotation.syntax(), VisibilitySyntax::Crate)
    {
        return Ok(false);
    }
    let reach_requirements = ReachRequirements::for_analysis(
        ctx,
        resolved_facade_reach(parent_facade_analysis),
        signature_exposure,
    );
    let resolved_chain_reach = reach_requirements.facade_reach();
    let required_reach = reach_requirements.required_reach();
    let exact_restricted_boundary =
        resolved_chain_reach
            .zip(required_reach)
            .and_then(|(_, required)| {
                (!reach_is_pub_crate(required, ctx) && required.to_source(ctx.tcx) != "pub")
                    .then_some(required)
            });
    let repair_context = match exact_restricted_boundary {
        Some(required) => OverbroadPubCrateRepairContext::ExactRestrictedBoundary { required },
        None if matches!(annotation.syntax(), VisibilitySyntax::InCrate)
            && required_reach.is_some_and(|required| reach_is_pub_crate(required, ctx)) =>
        {
            OverbroadPubCrateRepairContext::CanonicalPubCrate
        },
        None => OverbroadPubCrateRepairContext::PolicyFallback,
    };
    let advice = overbroad_pub_crate_advice(
        ctx,
        item,
        annotation,
        OverbroadPubCrateAdviceInput {
            finding_context,
            parent_facade_analysis,
            signature_exposure,
            repair_context,
            sink,
        },
    );
    let finding = source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        advice.into_finding_params(ctx, item, annotation),
    )?;
    sink.findings.push(finding);
    record_visibility_constraint(
        ctx,
        item,
        annotation,
        VisibilityConstraintInput {
            diagnostic_code: DiagnosticCode::OverbroadPubCrate,
            parent_facade_analysis,
            signature_exposure,
            outcome: StoredConstraintOutcome::Finding,
        },
        sink,
    );
    Ok(true)
}

struct OverbroadPubCrateAdvice {
    message:                 String,
    suggestion:              String,
    restricted_boundary:     Option<String>,
    exact_boundary_spelling: ExactBoundarySpelling,
}

impl OverbroadPubCrateAdvice {
    const fn manual(message: String, suggestion: String) -> Self {
        Self {
            message,
            suggestion,
            restricted_boundary: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        }
    }

    fn into_finding_params(
        self,
        ctx: &VisibilityContext<'_, '_>,
        item: &ItemInfo<'_>,
        annotation: &VisibilityAnnotation<'_>,
    ) -> FindingParams {
        let (fix_support, visibility_annotation, item_def_path, narrower_scope_def_path) =
            self.restricted_boundary.map_or_else(
                || (FixSupport::None, None, None, None),
                |boundary| {
                    (
                        FixSupport::RestrictedAnnotation,
                        Some(annotation.source().to_string()),
                        Some(use_sites::def_path_string(ctx.tcx, item.def_id)),
                        Some(boundary),
                    )
                },
            );
        FindingParams {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::OverbroadPubCrate,
            item: None,
            message: self.message,
            suggestion: Some(self.suggestion),
            fix_support,
            related: None,
            visibility_annotation,
            item_def_path,
            narrower_scope_def_path,
            exact_boundary_spelling: self.exact_boundary_spelling,
        }
    }
}

enum OverbroadPubCrateNoFacadeAdvice {
    SuggestionReason(OverbroadPubCrateSuggestionReason),
    ParentBoundary { def_path: String },
    RestrictedBoundary { def_path: String },
}

#[derive(Clone, Copy)]
enum OverbroadPubCrateRepairContext {
    ExactRestrictedBoundary { required: VisibilityReach },
    CanonicalPubCrate,
    PolicyFallback,
}

#[derive(Clone, Copy)]
struct OverbroadPubCrateAdviceInput<'borrow, 'facade> {
    finding_context:        &'borrow VisibilityFindingContext,
    parent_facade_analysis: Option<&'borrow ParentFacadeAnalysis<'facade>>,
    signature_exposure:     SignatureExposure,
    repair_context:         OverbroadPubCrateRepairContext,
    sink:                   &'borrow FindingsSink,
}

fn overbroad_pub_crate_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    input: OverbroadPubCrateAdviceInput<'_, '_>,
) -> OverbroadPubCrateAdvice {
    let OverbroadPubCrateAdviceInput {
        finding_context,
        parent_facade_analysis,
        signature_exposure,
        repair_context,
        sink,
    } = input;
    if signature_exposure.reaches_outside_crate() {
        let suggestion_reason = OverbroadPubCrateSuggestionReason::PublicSignatureExposure;
        let generic_message = format!(
            "`{}` is narrower than the item's required public reach",
            annotation.display_source()
        );
        return OverbroadPubCrateAdvice::manual(
            suggestion_reason.headline(generic_message),
            policy::overbroad_pub_crate_suggestion(suggestion_reason),
        );
    }

    let fallback = || match repair_context {
        OverbroadPubCrateRepairContext::ExactRestrictedBoundary { required } => {
            let message = match annotation.syntax() {
                VisibilitySyntax::Crate => {
                    String::from("use of `pub(crate)` does not match the parent facade boundary")
                },
                VisibilitySyntax::InCrate => {
                    String::from("`pub(in crate)` is wider than the exact parent facade boundary")
                },
                _ => format!("`{}` is broader than required", annotation.display_source()),
            };
            OverbroadPubCrateAdvice::manual(
                message,
                policy::consider_using(&required.to_source(ctx.tcx)),
            )
        },
        OverbroadPubCrateRepairContext::CanonicalPubCrate => OverbroadPubCrateAdvice::manual(
            String::from("`pub(in crate)` is a redundant spelling of `pub(crate)`"),
            policy::consider_using("pub(crate)"),
        ),
        OverbroadPubCrateRepairContext::PolicyFallback => {
            let no_facade_advice = match (signature_exposure, parent_facade_analysis) {
                (_, Some(parent_facade_analysis)) => {
                    OverbroadPubCrateNoFacadeAdvice::SuggestionReason(
                        overbroad_pub_crate_parent_facade_reason(
                            ctx,
                            parent_facade_analysis,
                            finding_context.module_location,
                        ),
                    )
                },
                (signature_exposure, None) => overbroad_pub_crate_no_facade_advice(
                    ctx,
                    item,
                    signature_exposure,
                    finding_context.module_location,
                    sink,
                ),
            };
            let generic_message =
                format!("`{}` is broader than required", annotation.display_source());
            match no_facade_advice {
                OverbroadPubCrateNoFacadeAdvice::SuggestionReason(suggestion_reason) => {
                    OverbroadPubCrateAdvice::manual(
                        suggestion_reason.headline(generic_message),
                        policy::overbroad_pub_crate_suggestion(suggestion_reason),
                    )
                },
                OverbroadPubCrateNoFacadeAdvice::ParentBoundary { def_path } => {
                    OverbroadPubCrateAdvice {
                        message:                 generic_message,
                        suggestion:              policy::consider_using("pub(super)"),
                        restricted_boundary:     Some(def_path),
                        exact_boundary_spelling: ExactBoundarySpelling::Parent,
                    }
                },
                OverbroadPubCrateNoFacadeAdvice::RestrictedBoundary { def_path } => {
                    let boundary_path = policy::crate_rooted_def_path(&def_path);
                    OverbroadPubCrateAdvice {
                        message:                 generic_message,
                        suggestion:              policy::consider_using(&format!(
                            "pub(in {boundary_path})"
                        )),
                        restricted_boundary:     Some(def_path),
                        exact_boundary_spelling: ExactBoundarySpelling::CratePath,
                    }
                },
            }
        },
    };
    parent_facade_blocker_text(ctx, parent_facade_analysis).map_or_else(fallback, |suggestion| {
        OverbroadPubCrateAdvice::manual(
            format!("`{}` is broader than required", annotation.display_source()),
            suggestion,
        )
    })
}

fn overbroad_pub_crate_parent_facade_reason(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: &ParentFacadeAnalysis<'_>,
    module_location: ModuleLocation,
) -> OverbroadPubCrateSuggestionReason {
    let parent_facade_reach = parent_facade_reach(parent_facade_analysis, ctx);
    if parent_facade_reach.parent_reach == ParentReach::RestrictedToChild {
        return OverbroadPubCrateSuggestionReason::LocationPolicy { module_location };
    }
    let boundary_path = resolved_facade_boundary_path(parent_facade_analysis, ctx);
    if parent_facade_reach.spelling == ParentFacadeSpelling::Super
        && parent_facade_reach.spelling_agreement == FacadeSpellingAgreement::Consistent
    {
        return boundary_path.map_or(
            OverbroadPubCrateSuggestionReason::ExactPubSuperParentFacadeWithoutKnownBoundary,
            |boundary_path| OverbroadPubCrateSuggestionReason::ExactPubSuperParentFacade {
                boundary_path,
            },
        );
    }
    boundary_path.map_or(
        OverbroadPubCrateSuggestionReason::ParentFacadeWithoutKnownBoundary,
        |boundary_path| OverbroadPubCrateSuggestionReason::ParentFacade { boundary_path },
    )
}

fn overbroad_pub_crate_no_facade_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    signature_exposure: SignatureExposure,
    module_location: ModuleLocation,
    sink: &FindingsSink,
) -> OverbroadPubCrateNoFacadeAdvice {
    let item_def_path = use_sites::def_path_string(ctx.tcx, item.def_id);
    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    let callers = current_pass_callers(sink, &item_def_path);
    let parent_scope = policy::parent_scope_def_path(&item_module);
    let caller_repair = policy::classify_no_facade_callers(&item_module, parent_scope, &callers);
    if facade_less_annotation_repair_is_supported(ctx, item.def_id) {
        if matches!(caller_repair, NoFacadeVisibilityRepair::UseParentVisibility)
            && !parent_scope.is_empty()
            && parent_scope != "crate"
        {
            return OverbroadPubCrateNoFacadeAdvice::ParentBoundary {
                def_path: parent_scope.to_string(),
            };
        }
        if matches!(
            caller_repair,
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
        ) && matches!(
            ctx.settings.visibility_config.pub_in_path,
            PubInPath::Permitted | PubInPath::Required
        ) && let Some(def_path) = policy::common_ancestor_def_path(&item_module, &callers)
        {
            return OverbroadPubCrateNoFacadeAdvice::RestrictedBoundary { def_path };
        }
    }
    let SignatureExposure::ExposedAt(signature_exposure) = signature_exposure else {
        return OverbroadPubCrateNoFacadeAdvice::SuggestionReason(
            OverbroadPubCrateSuggestionReason::LocationPolicy { module_location },
        );
    };
    let Some(signature_boundary_path) = visibility_reach_boundary_path(signature_exposure, ctx)
    else {
        return OverbroadPubCrateNoFacadeAdvice::SuggestionReason(
            OverbroadPubCrateSuggestionReason::LocationPolicy { module_location },
        );
    };
    let repair = merge_no_facade_signature_reach(
        ctx,
        item,
        caller_repair,
        SignatureExposure::ExposedAt(signature_exposure),
    );
    let boundary_path = match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            visibility_reach_boundary_path(required, ctx).unwrap_or(signature_boundary_path)
        },
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => {
            caller_boundary_path(&item_module, &callers)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility => signature_boundary_path,
    };
    OverbroadPubCrateNoFacadeAdvice::SuggestionReason(
        OverbroadPubCrateSuggestionReason::NoFacadeRepair {
            boundary_path,
            item_def_path: display_def_path(ctx, item),
            repair,
        },
    )
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let reach_requirements = ReachRequirements::for_analysis(
        ctx,
        resolved_facade_reach(parent_facade_analysis),
        signature_exposure,
    );
    if exact_pub_in_boundary_is_allowed(ctx, item, annotation, &reach_requirements, sink) {
        record_visibility_constraint(
            ctx,
            item,
            annotation,
            VisibilityConstraintInput {
                diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
                parent_facade_analysis,
                signature_exposure,
                outcome: StoredConstraintOutcome::Accepted,
            },
            sink,
        );
        return Ok(false);
    }

    if !matches!(
        annotation.syntax(),
        VisibilitySyntax::InParent | VisibilitySyntax::InCurrent | VisibilitySyntax::InPath(_)
    ) {
        return Ok(false);
    }
    let ForbiddenPubInText {
        message,
        suggestion,
        note,
    } = forbidden_pub_in_advice(
        ctx,
        item,
        annotation,
        finding_context,
        parent_facade_analysis,
        signature_exposure,
        sink,
    )
    .into_finding_text();
    let finding = source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
            item: None,
            message,
            suggestion,
            fix_support: FixSupport::None,
            related: note,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?;
    sink.findings.push(finding);
    record_visibility_constraint(
        ctx,
        item,
        annotation,
        VisibilityConstraintInput {
            diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
            parent_facade_analysis,
            signature_exposure,
            outcome: StoredConstraintOutcome::Finding,
        },
        sink,
    );
    Ok(true)
}

fn public_signature_pub_in_advice(annotation: &VisibilityAnnotation<'_>) -> ForbiddenPubInAdvice {
    let suggestion_reason = OverbroadPubCrateSuggestionReason::PublicSignatureExposure;
    let generic_message = policy::forbidden_pub_in_headline(&annotation.display_source());
    ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
        message:    suggestion_reason.headline(generic_message),
        suggestion: policy::overbroad_pub_crate_suggestion(suggestion_reason),
    }
}

#[derive(Clone, Copy)]
enum CanonicalPubInSpelling {
    Parent,
    Current,
}

/// The three text slots a `forbidden_pub_in_crate` finding fills: the headline,
/// the `| help:` line beneath the span, and the `= note:` line under both.
struct ForbiddenPubInText {
    message:    String,
    suggestion: Option<String>,
    note:       Option<String>,
}

enum ForbiddenPubInAdvice {
    NoSuggestion {
        message: String,
    },
    SuggestionWithoutCallerRefinement {
        message:    String,
        suggestion: String,
    },
    SuggestionWithCallerRefinement {
        message:                 String,
        suggestion:              String,
        item_def_path:           String,
        narrower_scope_def_path: String,
    },
    StructuralSuggestionControlledBySignatureReach {
        message:    String,
        suggestion: String,
    },
    /// Any of the above, plus the note saying why it is being offered.
    ///
    /// A wrapper rather than a field on each variant: only the repairs that ask
    /// for a re-export have something to explain, and the rest would carry a
    /// permanently empty slot.
    Explained {
        advice: Box<Self>,
        note:   String,
    },
}

impl ForbiddenPubInAdvice {
    /// Attaches the note saying why this repair is the one being offered, when
    /// there is a gap to describe.
    fn explained_by(self, note: Option<String>) -> Self {
        match note {
            Some(note) => Self::Explained {
                advice: Box::new(self),
                note,
            },
            None => self,
        }
    }

    fn into_finding_text(self) -> ForbiddenPubInText {
        match self {
            Self::NoSuggestion { message } => ForbiddenPubInText {
                message,
                suggestion: None,
                note: None,
            },
            Self::SuggestionWithoutCallerRefinement {
                message,
                suggestion,
            }
            | Self::SuggestionWithCallerRefinement {
                message,
                suggestion,
                item_def_path: _,
                narrower_scope_def_path: _,
            }
            | Self::StructuralSuggestionControlledBySignatureReach {
                message,
                suggestion,
            } => ForbiddenPubInText {
                message,
                suggestion: Some(suggestion),
                note: None,
            },
            Self::Explained { advice, note } => ForbiddenPubInText {
                note: Some(note),
                ..advice.into_finding_text()
            },
        }
    }

    fn prepend_repair(self, preceding_repair: String) -> Self {
        match self {
            Self::NoSuggestion { message } => Self::SuggestionWithoutCallerRefinement {
                message,
                suggestion: preceding_repair,
            },
            Self::SuggestionWithoutCallerRefinement {
                message,
                suggestion,
            } => Self::SuggestionWithoutCallerRefinement {
                message,
                suggestion: format!("{preceding_repair} — {suggestion}"),
            },
            Self::SuggestionWithCallerRefinement {
                message,
                suggestion,
                item_def_path,
                narrower_scope_def_path,
            } => Self::SuggestionWithCallerRefinement {
                message,
                suggestion: format!("{preceding_repair} — {suggestion}"),
                item_def_path,
                narrower_scope_def_path,
            },
            Self::StructuralSuggestionControlledBySignatureReach {
                message,
                suggestion,
            } => Self::StructuralSuggestionControlledBySignatureReach {
                message,
                suggestion: format!("{preceding_repair} — {suggestion}"),
            },
            Self::Explained { advice, note } => Self::Explained {
                advice: Box::new(advice.prepend_repair(preceding_repair)),
                note,
            },
        }
    }
}

fn forbidden_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &FindingsSink,
) -> ForbiddenPubInAdvice {
    let annotation_reach = annotation.reach(item.def_id, ctx.tcx);
    let public_signature_bypasses_blocker = signature_exposure.reaches_outside_crate()
        && parent_facade_analysis.is_some_and(|analysis| {
            matches!(analysis.chain, FacadeChainResolution::Unresolvable { .. })
        });
    if public_signature_bypasses_blocker {
        return public_signature_pub_in_advice(annotation);
    }
    match parent_facade_analysis.map(|analysis| analysis.chain) {
        Some(FacadeChainResolution::Unresolvable {
            blocker: FacadeChainBlocker::Glob(_),
        }) => glob_blocked_pub_in_advice(ctx, parent_facade_analysis),
        Some(FacadeChainResolution::Unresolvable { .. }) => unresolvable_pub_in_advice(
            ctx,
            item,
            annotation,
            annotation_reach,
            parent_facade_analysis,
            signature_exposure,
        ),
        Some(FacadeChainResolution::Resolved { required }) => resolved_pub_in_advice(
            ctx,
            item,
            annotation,
            annotation_reach,
            required,
            signature_exposure,
        ),
        None => no_facade_pub_in_advice(
            ctx,
            item,
            annotation,
            annotation_reach,
            finding_context,
            signature_exposure,
            sink,
        ),
    }
}

enum ReachRequirements {
    Facade {
        facade:   VisibilityReach,
        required: VisibilityReach,
    },
    SignatureOnly {
        required: VisibilityReach,
    },
    Unconstrained,
}

impl ReachRequirements {
    fn for_analysis(
        ctx: &VisibilityContext<'_, '_>,
        facade_reach: Option<VisibilityReach>,
        signature_exposure: SignatureExposure,
    ) -> Self {
        facade_reach.map_or_else(
            || match signature_exposure {
                SignatureExposure::ExposedAt(required) => Self::SignatureOnly { required },
                SignatureExposure::Contained => Self::Unconstrained,
            },
            |facade| Self::Facade {
                facade,
                required: signature_exposure.combined_with(facade, ctx),
            },
        )
    }

    const fn facade_reach(&self) -> Option<VisibilityReach> {
        match self {
            Self::Facade {
                facade,
                required: _,
            } => Some(*facade),
            Self::SignatureOnly { required: _ } | Self::Unconstrained => None,
        }
    }

    const fn required_reach(&self) -> Option<VisibilityReach> {
        match self {
            Self::Facade {
                facade: _,
                required,
            }
            | Self::SignatureOnly { required } => Some(*required),
            Self::Unconstrained => None,
        }
    }

    fn exact_facade_boundary_matches(
        &self,
        annotation_reach: VisibilityReach,
        ctx: &VisibilityContext<'_, '_>,
    ) -> bool {
        match self {
            Self::Facade {
                facade: _,
                required,
            } => annotation_reach.compare(*required, ctx.tcx) == Some(Ordering::Equal),
            Self::SignatureOnly { required: _ } | Self::Unconstrained => false,
        }
    }
}

/// Whether `pub_in_path` accepts this annotation as written.
///
/// Both `permitted` and `required` — the default — accept an exact,
/// crate-rooted boundary; only `forbidden` refuses one. A `use` item earns that
/// acceptance on the same terms a declaration does: the spelling is the whole
/// complaint, and a setting that permits the spelling answers it. Only the
/// caller-derived arm below stays closed to `use` items, which
/// [`facade_less_boundary_matches_callers`] enforces for itself; the arm a
/// `use` item does earn is [`pub_in_boundary_is_the_items_own_module`]. Modules
/// are left out: `pub mod` answers to its own policy.
fn exact_pub_in_boundary_is_allowed(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    reach_requirements: &ReachRequirements,
    sink: &FindingsSink,
) -> bool {
    matches!(
        ctx.settings.visibility_config.pub_in_path,
        PubInPath::Permitted | PubInPath::Required
    ) && matches!(item.category, ItemCategory::Declaration | ItemCategory::Use)
        && matches!(
            annotation.syntax(),
            VisibilitySyntax::InPath(PathSpelling::CrateRooted)
        )
        && (reach_requirements
            .exact_facade_boundary_matches(annotation.reach(item.def_id, ctx.tcx), ctx)
            || facade_less_boundary_matches_callers(ctx, item, annotation, sink)
            || pub_in_boundary_is_the_items_own_module(ctx, item, annotation))
}

/// Whether the boundary the annotation names is the module the `use` item is
/// already declared in.
///
/// [`facade_less_boundary_matches_callers`] refuses every `use` item, because
/// resolved paths name the imported target rather than the alias, so
/// [`no_facade_pub_in_advice`] answers with [`policy::add_facade_suggestion`] —
/// which, for an alias already declared at the boundary, asks for a re-export
/// of the item into the module holding the line being reported. There is
/// nothing for that re-export to add: `pub(in P)` on an item in `P` reaches the
/// whole `P` subtree, which is exactly where a name at `P` is already visible.
fn pub_in_boundary_is_the_items_own_module(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
) -> bool {
    if item.category != ItemCategory::Use {
        return false;
    }
    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    policy::canonical_pub_in_boundary(&item_module, annotation.source())
        == Some(policy::crate_rooted_def_path(&item_module))
}

fn facade_less_boundary_matches_callers(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    sink: &FindingsSink,
) -> bool {
    if item.category == ItemCategory::Use {
        // `facade_less_boundary_matches_callers` rejects `ItemCategory::Use`:
        // resolved paths name the imported target, not the local `use` item, so
        // the caller set gathered below describes the target. It cannot show
        // where the alias is used, and so cannot show that this boundary covers
        // them.
        return false;
    }
    if !facade_less_annotation_repair_is_supported(ctx, item.def_id) {
        return false;
    }
    let item_def_path = use_sites::def_path_string(ctx.tcx, item.def_id);
    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    let callers = current_pass_callers(sink, &item_def_path);
    if !matches!(
        policy::classify_no_facade_callers(
            &item_module,
            policy::parent_scope_def_path(&item_module),
            &callers,
        ),
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
    ) {
        return false;
    }
    let Some(boundary) = policy::common_ancestor_def_path(&item_module, &callers) else {
        return false;
    };
    policy::canonical_pub_in_boundary(&item_module, annotation.source())
        == Some(policy::crate_rooted_def_path(&boundary))
}

/// Whether source this compilation's `#[cfg]` configuration left out writes
/// the item's name.
///
/// [`collect_use_sites`] walks the post-expansion HIR, so a consumer inside an
/// excluded region contributes no use site.
/// `VisibilityConstraintGroup::caller_required_reach` then reads a caller set
/// missing that consumer's module and resolves the boundary to what the
/// remaining callers need. Rust accepts no `#[cfg]` on a visibility qualifier,
/// so the narrowed spelling satisfies one configuration and stops the other
/// compiling; `overbroad-pub-crate` reports nothing here rather than advise it.
///
/// A tuple field's name is its index, which only `.0` access writes. Building
/// one with `Failure(code)` or matching `Failure(code)` writes the struct's
/// name instead, so a tuple field also counts as named wherever its struct is.
///
/// [`collect_use_sites`]: crate::compiler::visibility::use_sites::collect_use_sites
fn cfg_excluded_source_names_item(ctx: &VisibilityContext<'_, '_>, item: &ItemInfo<'_>) -> bool {
    let excluded_source_names =
        |name: &str| ctx.cfg_excluded_references.reference(name) == CfgExcludedReference::Present;
    item.name.is_some_and(excluded_source_names)
        || tuple_struct_name(ctx.tcx, item.def_id)
            .is_some_and(|struct_name| excluded_source_names(struct_name.as_str()))
}

/// The name of the tuple struct that declares `def_id` as one of its fields.
fn tuple_struct_name(tcx: TyCtxt<'_>, def_id: LocalDefId) -> Option<Symbol> {
    if tcx.def_kind(def_id) != DefKind::Field {
        return None;
    }
    let parent = tcx.parent(def_id.to_def_id());
    if tcx.def_kind(parent) != DefKind::Struct {
        return None;
    }
    (tcx.adt_def(parent).non_enum_variant().ctor_kind() == Some(CtorKind::Fn))
        .then(|| tcx.item_name(parent))
}

fn facade_is_possible(ctx: &VisibilityContext<'_, '_>, def_id: LocalDefId) -> bool {
    if matches!(
        ctx.tcx.def_kind(def_id),
        DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy | DefKind::Field
    ) {
        return false;
    }
    matches!(
        ctx.tcx.def_kind(ctx.tcx.parent(def_id.to_def_id())),
        DefKind::Mod
    )
}

fn facade_less_annotation_repair_is_supported(
    ctx: &VisibilityContext<'_, '_>,
    def_id: LocalDefId,
) -> bool {
    !facade_is_possible(ctx, def_id)
}

fn joined_required_reach(
    ctx: &VisibilityContext<'_, '_>,
    facade_reach: Option<VisibilityReach>,
    signature_exposure: SignatureExposure,
) -> Option<VisibilityReach> {
    match (facade_reach, signature_exposure) {
        (Some(facade_reach), signature_exposure) => {
            Some(signature_exposure.combined_with(facade_reach, ctx))
        },
        (None, SignatureExposure::ExposedAt(signature_reach)) => Some(signature_reach),
        (None, SignatureExposure::Contained) => None,
    }
}

fn merge_no_facade_signature_reach(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    caller_repair: NoFacadeVisibilityRepair,
    signature_exposure: SignatureExposure,
) -> NoFacadeVisibilityRepair {
    let caller_reach = match caller_repair {
        NoFacadeVisibilityRepair::RemoveAnnotation => {
            VisibilityAnnotation::Private.reach(item.def_id, ctx.tcx)
        },
        NoFacadeVisibilityRepair::UseParentVisibility => {
            VisibilityAnnotation::Parent.reach(item.def_id, ctx.tcx)
        },
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => {
            let repair = match signature_exposure {
                SignatureExposure::ExposedAt(required_reach)
                    if !signature_exposure.is_satisfied_by(
                        VisibilityAnnotation::Parent.reach(item.def_id, ctx.tcx),
                        ctx,
                    ) =>
                {
                    NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach {
                        required: required_reach,
                    }
                },
                SignatureExposure::ExposedAt(_) | SignatureExposure::Contained => {
                    NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
                },
            };
            return repair;
        },
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            return NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required };
        },
    };
    let required_reach = signature_exposure.combined_with(caller_reach, ctx);
    let private_reach = VisibilityAnnotation::Private.reach(item.def_id, ctx.tcx);
    if matches!(
        private_reach.compare(required_reach, ctx.tcx),
        Some(Ordering::Equal | Ordering::Greater)
    ) {
        return NoFacadeVisibilityRepair::RemoveAnnotation;
    }
    let parent_reach = VisibilityAnnotation::Parent.reach(item.def_id, ctx.tcx);
    if matches!(
        parent_reach.compare(required_reach, ctx.tcx),
        Some(Ordering::Equal | Ordering::Greater)
    ) {
        return NoFacadeVisibilityRepair::UseParentVisibility;
    }
    NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach {
        required: required_reach,
    }
}

fn default_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
) -> ForbiddenPubInAdvice {
    let message = format!(
        "use of `{}` is forbidden by policy",
        annotation.display_source()
    );
    match annotation.syntax() {
        VisibilitySyntax::InParent => ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message,
            suggestion: policy::consider_using("pub(super)"),
        },
        VisibilitySyntax::InCurrent => ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message,
            suggestion: policy::consider_using("pub(self)"),
        },
        VisibilitySyntax::InPath(PathSpelling::Relative) => {
            ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
                message,
                suggestion: policy::consider_using(&annotation_reach.to_source(ctx.tcx)),
            }
        },
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Crate
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InPath(PathSpelling::CrateRooted) => {
            ForbiddenPubInAdvice::NoSuggestion { message }
        },
    }
}

fn glob_blocked_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> ForbiddenPubInAdvice {
    let message = String::from("parent facade does not provide a resolvable visibility boundary");
    match parent_facade_blocker_text(ctx, parent_facade_analysis) {
        Some(text) => ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message,
            suggestion: format!("{text} before using `pub(in ...)`"),
        },
        None => ForbiddenPubInAdvice::NoSuggestion { message },
    }
}

fn unresolvable_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
) -> ForbiddenPubInAdvice {
    let advice = match annotation.syntax() {
        VisibilitySyntax::InParent => canonical_pub_in_spelling_advice(
            ctx,
            item,
            CanonicalPubInSpelling::Parent,
            signature_exposure,
        ),
        VisibilitySyntax::InCurrent => canonical_pub_in_spelling_advice(
            ctx,
            item,
            CanonicalPubInSpelling::Current,
            signature_exposure,
        ),
        _ => default_pub_in_advice(ctx, annotation, annotation_reach),
    };
    match parent_facade_blocker_text(ctx, parent_facade_analysis) {
        Some(blocker_text) => advice.prepend_repair(blocker_text),
        None => advice,
    }
}

fn canonical_pub_in_spelling_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    spelling: CanonicalPubInSpelling,
    signature_exposure: SignatureExposure,
) -> ForbiddenPubInAdvice {
    let (original_source, canonical_annotation) = match spelling {
        CanonicalPubInSpelling::Parent => ("pub(in super)", VisibilityAnnotation::Parent),
        CanonicalPubInSpelling::Current => ("pub(in self)", VisibilityAnnotation::Current),
    };
    let canonical_reach = canonical_annotation.reach(item.def_id, ctx.tcx);
    if let SignatureExposure::ExposedAt(required) = signature_exposure
        && !signature_exposure.is_satisfied_by(canonical_reach, ctx)
    {
        let boundary_path = visibility_reach_boundary_path(required, ctx)
            .unwrap_or_else(|| String::from("crate-external"));
        let repair = NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required };
        return ForbiddenPubInAdvice::StructuralSuggestionControlledBySignatureReach {
            message:    format!(
                "`{}` would be narrower than this item's required signature reach at \
                 `{boundary_path}`",
                canonical_annotation.source()
            ),
            suggestion: policy::no_facade_suggestion(
                repair,
                &boundary_path,
                &use_sites::def_path_string(ctx.tcx, item.def_id),
            ),
        };
    }
    ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
        message:    format!(
            "`{original_source}` is a redundant spelling of `{}`",
            canonical_annotation.source()
        ),
        suggestion: policy::consider_using(canonical_annotation.source()),
    }
}

/// The advice selected by the resolved boundary and the project's spelling policy.
enum RestrictedVisibilityAdviceCase {
    CrateBoundaryExact,
    ForbiddenExactPath,
    ExactRestrictedSpelling { required: VisibilityReach },
    SignatureRequiresWider { required: VisibilityReach },
    AnnotationTooWide { required: VisibilityReach },
    BoundaryMismatch { required: VisibilityReach },
}

impl RestrictedVisibilityAdviceCase {
    fn for_boundary(
        ctx: &VisibilityContext<'_, '_>,
        item: &ItemInfo<'_>,
        annotation: &VisibilityAnnotation<'_>,
        annotation_reach: VisibilityReach,
        facade_reach: VisibilityReach,
        required: VisibilityReach,
    ) -> Self {
        match annotation_reach.compare(required, ctx.tcx) {
            Some(Ordering::Equal) => {
                if reach_is_pub_crate(required, ctx) {
                    return Self::CrateBoundaryExact;
                }
                match (
                    annotation.syntax(),
                    item.category,
                    ctx.settings.visibility_config.pub_in_path,
                ) {
                    (
                        VisibilitySyntax::InPath(PathSpelling::CrateRooted),
                        ItemCategory::Declaration | ItemCategory::Use,
                        PubInPath::Forbidden,
                    ) => Self::ForbiddenExactPath,
                    _ => Self::ExactRestrictedSpelling { required },
                }
            },
            comparison => match (annotation_reach.compare(facade_reach, ctx.tcx), comparison) {
                (Some(Ordering::Equal), _) => Self::SignatureRequiresWider { required },
                (_, Some(Ordering::Greater)) => Self::AnnotationTooWide { required },
                _ => Self::BoundaryMismatch { required },
            },
        }
    }
}

fn resolved_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    facade_reach: VisibilityReach,
    signature_exposure: SignatureExposure,
) -> ForbiddenPubInAdvice {
    let required = signature_exposure.combined_with(facade_reach, ctx);
    let advice_case = RestrictedVisibilityAdviceCase::for_boundary(
        ctx,
        item,
        annotation,
        annotation_reach,
        facade_reach,
        required,
    );
    let (message, required) = match advice_case {
        RestrictedVisibilityAdviceCase::CrateBoundaryExact => {
            return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
                message:    String::from("parent facade caps reach at `pub(crate)`"),
                suggestion: policy::consider_using("pub(crate)"),
            };
        },
        RestrictedVisibilityAdviceCase::ForbiddenExactPath => {
            // The annotation already sits at the exact boundary. The spelling is
            // the entire complaint and the setting that permits it is the whole
            // answer. A declaration can also widen to `pub` and let the facade cap
            // the reach; a `use` item is not offered that, because a `pub use` of a
            // narrower item is E0364 and proving the target wide enough is not work
            // this function does.
            let suggestion = match item.category {
                ItemCategory::Declaration => format!(
                    "{}; or {}",
                    policy::consider_using("pub"),
                    policy::permit_pub_in_path()
                ),
                ItemCategory::Module | ItemCategory::Use => policy::permit_pub_in_path(),
            };
            return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
                message: format!(
                    "use of `{}` is disabled by project visibility policy",
                    annotation.display_source()
                ),
                suggestion,
            };
        },
        RestrictedVisibilityAdviceCase::ExactRestrictedSpelling { required } => {
            return exact_restricted_spelling_advice(ctx, annotation, required);
        },
        RestrictedVisibilityAdviceCase::SignatureRequiresWider { required } => (
            format!(
                "signature exposure requires the wider `{}` annotation",
                required.to_source(ctx.tcx)
            ),
            required,
        ),
        RestrictedVisibilityAdviceCase::AnnotationTooWide { required } => (
            format!(
                "`{}` is wider than the exact parent facade boundary",
                annotation.display_source()
            ),
            required,
        ),
        RestrictedVisibilityAdviceCase::BoundaryMismatch { required } => (
            format!(
                "use of `{}` does not match the parent facade boundary",
                annotation.display_source()
            ),
            required,
        ),
    };
    ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
        message,
        suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
    }
}

fn exact_restricted_spelling_advice(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    required: VisibilityReach,
) -> ForbiddenPubInAdvice {
    match annotation.syntax() {
        VisibilitySyntax::InParent => ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    String::from("`pub(in super)` is a redundant spelling of `pub(super)`"),
            suggestion: policy::consider_using("pub(super)"),
        },
        VisibilitySyntax::InCurrent => ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    String::from("`pub(in self)` is a redundant spelling of `pub(self)`"),
            suggestion: policy::consider_using("pub(self)"),
        },
        VisibilitySyntax::InPath(PathSpelling::Relative) => {
            ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
                message:    format!(
                    "use of `{}` does not use the canonical crate-rooted boundary",
                    annotation.display_source()
                ),
                suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
            }
        },
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Crate
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InPath(PathSpelling::CrateRooted) => {
            ForbiddenPubInAdvice::NoSuggestion {
                message: format!(
                    "use of `{}` is forbidden by policy",
                    annotation.display_source()
                ),
            }
        },
    }
}

fn no_facade_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    finding_context: &VisibilityFindingContext,
    signature_exposure: SignatureExposure,
    sink: &FindingsSink,
) -> ForbiddenPubInAdvice {
    if signature_exposure.reaches_outside_crate() {
        let suggestion_reason = OverbroadPubCrateSuggestionReason::PublicSignatureExposure;
        let generic_message = policy::forbidden_pub_in_headline(&annotation.display_source());
        return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    suggestion_reason.headline(generic_message),
            suggestion: policy::overbroad_pub_crate_suggestion(suggestion_reason),
        };
    }

    match annotation.syntax() {
        VisibilitySyntax::InParent => {
            return canonical_pub_in_spelling_advice(
                ctx,
                item,
                CanonicalPubInSpelling::Parent,
                signature_exposure,
            );
        },
        VisibilitySyntax::InCurrent => {
            return canonical_pub_in_spelling_advice(
                ctx,
                item,
                CanonicalPubInSpelling::Current,
                signature_exposure,
            );
        },
        _ => {},
    }

    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    let boundary_path = policy::canonical_pub_in_boundary(&item_module, annotation.source())
        .or_else(|| visibility_reach_boundary_path(annotation_reach, ctx))
        .unwrap_or_else(|| String::from("crate"));
    let item_def_path = use_sites::def_path_string(ctx.tcx, item.def_id);

    if item.category == ItemCategory::Use {
        // `no_facade_pub_in_advice` excludes caller-derived repairs for
        // `ItemCategory::Use`: resolved paths name the imported target, not the
        // local `use` item, so the caller set gathered below describes the target and cannot say
        // whether the alias still has users. That rules out every caller-derived
        // repair. The facade survives: `boundary_path` is read off the
        // annotation, so naming it asks nothing of the callers.
        let display_path = display_def_path(ctx, item);
        return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    policy::forbidden_pub_in_headline(annotation.source()),
            suggestion: policy::add_facade_suggestion(&boundary_path, &display_path),
        }
        .explained_by(policy::missing_facade_note(&boundary_path, &display_path));
    }

    let parent_scope = ParentScope::for_module(&item_module, finding_context.logical_module_depth);
    let callers = current_pass_callers(sink, &item_def_path);
    let caller_repair = parent_scope.classify_callers(&item_module, &callers);
    let repair = merge_no_facade_signature_reach(ctx, item, caller_repair, signature_exposure);
    let boundary_path = match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            visibility_reach_boundary_path(required, ctx).unwrap_or(boundary_path)
        },
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => {
            caller_boundary_path(&item_module, &callers)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility => boundary_path,
    };
    let message = policy::no_facade_headline(
        repair,
        policy::forbidden_pub_in_headline(annotation.source()),
    );
    let suggestion = policy::no_facade_suggestion(repair, &boundary_path, &item_def_path);
    // `policy::missing_facade_note` and `policy::no_facade_caller_note` split on
    // what the reader has to be convinced of. A re-export closes a gap between
    // reach and path, which needs both named. A
    // narrower visibility rests on where the item is used, which needs that
    // scope named instead.
    let note = match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
        | NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { .. } => {
            policy::missing_facade_note(&boundary_path, &item_def_path)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility => {
            parent_scope.caller_note(repair, &item_module)
        },
    };
    match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { .. } => {
            ForbiddenPubInAdvice::StructuralSuggestionControlledBySignatureReach {
                message,
                suggestion,
            }
            .explained_by(note)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility
        | NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => {
            ForbiddenPubInAdvice::SuggestionWithCallerRefinement {
                message,
                suggestion,
                item_def_path,
                narrower_scope_def_path: item_module,
            }
            .explained_by(note)
        },
    }
}

#[derive(Clone, Copy)]
enum ParentScope<'path> {
    CrateRoot,
    Enclosing { def_path: &'path str },
}

impl<'path> ParentScope<'path> {
    fn for_module(item_module: &'path str, logical_module_depth: usize) -> Self {
        match logical_module_depth {
            0 | 1 => Self::CrateRoot,
            2.. => Self::Enclosing {
                def_path: policy::parent_scope_def_path(item_module),
            },
        }
    }

    fn classify_callers(
        self,
        item_module: &str,
        callers: &BTreeSet<String>,
    ) -> NoFacadeVisibilityRepair {
        match self {
            // The policy API spells the crate-root scope as an empty path.
            Self::CrateRoot => policy::classify_no_facade_callers(item_module, "", callers),
            Self::Enclosing { def_path } => {
                policy::classify_no_facade_callers(item_module, def_path, callers)
            },
        }
    }

    fn caller_note(self, repair: NoFacadeVisibilityRepair, item_module: &str) -> Option<String> {
        match self {
            Self::CrateRoot => policy::no_facade_caller_note(repair, item_module, ""),
            Self::Enclosing { def_path } => {
                policy::no_facade_caller_note(repair, item_module, def_path)
            },
        }
    }
}

enum DisplayDefPath<'path> {
    AtRoot {
        name: &'path str,
    },
    WithinModule {
        module: &'path str,
        name:   &'path str,
    },
}

impl<'path> From<&'path str> for DisplayDefPath<'path> {
    fn from(def_path: &'path str) -> Self {
        match def_path.rsplit_once("::") {
            Some((module, name)) => Self::WithinModule { module, name },
            None => Self::AtRoot { name: def_path },
        }
    }
}

impl DisplayDefPath<'_> {
    fn with_item_name(self, item_name: Option<&str>) -> String {
        let name = match self {
            Self::AtRoot { name } | Self::WithinModule { module: _, name } => name,
        };
        let name = item_name.filter(|_| name.starts_with('{')).unwrap_or(name);
        match self {
            Self::AtRoot { name: _ } => String::from(name),
            Self::WithinModule { module, name: _ } => format!("{module}::{name}"),
        }
    }
}

/// The def path to print for `item`, as the reader would write it.
///
/// `def_path_str` renders a `use` binding as `{use#0}`: the import introduces a
/// name but no def-path segment carrying it. Every other item already spells
/// its own name, so only the import needs HIR's ident substituted back in.
/// Findings keep the unmodified def path, which is the key callers are indexed
/// under.
fn display_def_path(ctx: &VisibilityContext<'_, '_>, item: &ItemInfo<'_>) -> String {
    let def_path = use_sites::def_path_string(ctx.tcx, item.def_id);
    DisplayDefPath::from(def_path.as_str()).with_item_name(item.name)
}

fn current_pass_callers(sink: &FindingsSink, item_def_path: &str) -> BTreeSet<String> {
    sink.use_sites.callers(item_def_path)
}

fn caller_boundary_path(item_module: &str, callers: &BTreeSet<String>) -> String {
    policy::common_ancestor_def_path(item_module, callers).map_or_else(
        || String::from("crate"),
        |boundary| policy::crate_rooted_def_path(&boundary),
    )
}

fn record_review_pub_mod(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.category != ItemCategory::Module
        || matches!(annotation.syntax(), VisibilitySyntax::Private)
    {
        return Ok(());
    }
    // `record_review_pub_mod` accepts a crate-root `pub mod prelude;` when
    // `ctx.settings.visibility_config.prelude_pub_mod` is `PreludePubMod::Allowed`.
    if matches!(
        ctx.settings.visibility_config.prelude_pub_mod,
        PreludePubMod::Allowed
    ) && item.name == Some(PRELUDE_MODULE_NAME)
        && finding_context.module_location == ModuleLocation::CrateRoot
    {
        return Ok(());
    }
    let allowlisted = finding_context
        .config_rel_path
        .as_ref()
        .is_some_and(|path| {
            ctx.settings
                .visibility_config
                .allow_pub_mod
                .iter()
                .any(|allowed| allowed == path)
        });
    if allowlisted {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ReviewPubMod,
            item:                    item.name.map(str::to_owned),
            message:                 "`pub mod` requires explicit review or allowlisting"
                .to_string(),
            suggestion:              None,
            fix_support:             FixSupport::None,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?);
    Ok(())
}

fn maybe_record_narrow_to_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    if ctx.public_visibility_targets.contains(&item.def_id)
        || ctx
            .reexport_index
            .has_public_reexport(ctx.tcx, item.def_id, item.facade_subject)
    {
        return Ok(());
    }
    if ctx
        .effective_visibilities
        .is_public_at_level(item.def_id, Level::Reachable)
    {
        return Ok(());
    }
    let crate_reach = VisibilityAnnotation::Crate.reach(item.def_id, ctx.tcx);
    if !signature_exposure.is_satisfied_by(crate_reach, ctx) {
        return Ok(());
    }
    record_narrow_to_pub_crate(
        ctx,
        item,
        name,
        kind_label,
        "item is not re-exported by the crate root — use `pub(crate)`",
        sink,
    )
}

fn maybe_record_narrow_to_pub_crate_nested(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    let Some(parent_facade_analysis) = parent_facade_analysis else {
        return Ok(());
    };
    let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain else {
        return Ok(());
    };
    let combined_required_reach = signature_exposure.combined_with(required, ctx);
    let crate_reach = VisibilityAnnotation::Crate.reach(item.def_id, ctx.tcx);
    if parent_facade_analysis.nearest.spelling_agreement == FacadeSpellingAgreement::Conflicting
        || !reach_is_pub_crate(combined_required_reach, ctx)
        || !matches!(
            crate_reach.compare(combined_required_reach, ctx.tcx),
            Some(Ordering::Equal | Ordering::Greater)
        )
    {
        return Ok(());
    }
    record_narrow_to_pub_crate(
        ctx,
        item,
        name,
        kind_label,
        "parent facade caps reach at `pub(crate)` — narrow source to match",
        sink,
    )
}

/// Both narrow-to-`pub(crate)` paths report the same finding shape; only the
/// explanation of why the item can be narrowed differs.
fn record_narrow_to_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    name: &str,
    kind_label: &str,
    message: &str,
    sink: &mut FindingsSink,
) -> Result<()> {
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            diagnostic_code:         DiagnosticCode::NarrowToPubCrate,
            item:                    Some(StoredFinding::render_item(kind_label, name)),
            message:                 String::from(message),
            suggestion:              Some(policy::consider_using("pub(crate)")),
            fix_support:             FixSupport::NarrowToPubCrate,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?);
    Ok(())
}

const fn parent_facade_exports_item(
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> bool {
    parent_facade_analysis.is_some()
}

fn parent_facade_reach(
    parent_facade_analysis: &ParentFacadeAnalysis<'_>,
    ctx: &VisibilityContext<'_, '_>,
) -> ParentFacadeReach {
    let occurrence = parent_facade_analysis.nearest.selected;
    policy::parent_facade_reach_for_occurrence(
        ctx,
        occurrence,
        parent_facade_analysis.nearest.spelling_agreement,
    )
}

fn resolved_facade_boundary_path(
    parent_facade_analysis: &ParentFacadeAnalysis<'_>,
    ctx: &VisibilityContext<'_, '_>,
) -> Option<String> {
    let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain else {
        return None;
    };
    visibility_reach_boundary_path(required, ctx)
}

fn visibility_reach_boundary_path(
    reach: VisibilityReach,
    ctx: &VisibilityContext<'_, '_>,
) -> Option<String> {
    let source = reach.to_source(ctx.tcx);
    match source.as_str() {
        "pub" => Some(String::from("crate-external")),
        "pub(crate)" => Some(String::from("crate")),
        restricted => restricted
            .strip_prefix("pub(in ")
            .and_then(|path| path.strip_suffix(')'))
            .map(String::from),
    }
}

fn reach_is_pub_crate(reach: VisibilityReach, ctx: &VisibilityContext<'_, '_>) -> bool {
    let crate_reach = VisibilityReach::from(Visibility::Restricted(CRATE_DEF_ID.to_def_id()));
    reach.compare(crate_reach, ctx.tcx) == Some(Ordering::Equal)
}

fn parent_facade_blocker_text(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> Option<String> {
    let FacadeChainResolution::Unresolvable { ref blocker } = parent_facade_analysis?.chain else {
        return None;
    };
    let occurrence = blocker.occurrence();
    let path = source::real_file_path(ctx.tcx, occurrence.span)?;
    let relative_path = path
        .strip_prefix(ctx.source_root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let line = ctx
        .tcx
        .sess
        .source_map()
        .lookup_char_pos(occurrence.span.lo())
        .line;
    match blocker {
        FacadeChainBlocker::Glob(_) => Some(format!(
            "facade at {relative_path}:{line} uses `*`; replace it with an explicit re-export"
        )),
        FacadeChainBlocker::ForeignBoundary(_) => Some(format!(
            "facade chain leaves the crate at {relative_path}:{line}"
        )),
    }
}

fn facade_chain_fix_support(
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    fix_support: FixSupport,
) -> FixSupport {
    match parent_facade_analysis.map(|analysis| analysis.chain) {
        Some(FacadeChainResolution::Resolved { .. }) => fix_support,
        Some(FacadeChainResolution::Unresolvable { .. }) | None => FixSupport::None,
    }
}

fn maybe_record_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match policy::classify_suspicious_pub(ctx, input, parent_facade_analysis, signature_exposure)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
            record_internal_parent_facade_review(
                ctx,
                input,
                parent_facade_analysis,
                related,
                sink,
            )?;
        },
        SuspiciousPubAssessment::Warn {
            fix_support,
            related,
            stale_parent_pub_use,
        } => {
            record_suspicious_pub_warning(
                ctx,
                input,
                SuspiciousWarningInput {
                    kind_label,
                    annotation,
                    parent_facade_analysis,
                    signature_exposure,
                    warning: SuspiciousWarning {
                        fix_support,
                        related,
                        stale_parent_pub_use,
                    },
                },
                sink,
            )?;
        },
    }
    Ok(())
}

struct SuspiciousWarning {
    fix_support:          FixSupport,
    related:              Option<String>,
    stale_parent_pub_use: Option<ParentFacadeExportStatus>,
}

struct SuspiciousWarningInput<'borrow, 'syntax, 'facade> {
    kind_label:             &'borrow str,
    annotation:             &'borrow VisibilityAnnotation<'syntax>,
    parent_facade_analysis: Option<&'borrow ParentFacadeAnalysis<'facade>>,
    signature_exposure:     SignatureExposure,
    warning:                SuspiciousWarning,
}

/// A crate-relative restricted boundary that can be written by the annotation fixer.
struct ValidatedRestrictedBoundary(String);

impl ValidatedRestrictedBoundary {
    fn from_crate_rooted(boundary: &str) -> Option<Self> {
        let boundary = boundary.strip_prefix("crate::")?;
        if boundary.is_empty() {
            return None;
        }
        Some(Self(String::from(boundary)))
    }
}

enum NarrowingTarget {
    Exact(ValidatedRestrictedBoundary),
    CallerSuppression { enclosing_module: String },
}

impl NarrowingTarget {
    fn into_scope_def_path(self) -> String {
        match self {
            Self::Exact(boundary) => boundary.0,
            Self::CallerSuppression { enclosing_module } => enclosing_module,
        }
    }
}

enum SuspiciousPubAdvice {
    Narrowing {
        suggestion: String,
        target:     NarrowingTarget,
    },
    Structural {
        message:    String,
        suggestion: String,
    },
}

fn record_internal_parent_facade_review(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    related: Option<String>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(status) = policy::parent_facade_export_status(
        ctx,
        parent_facade_analysis,
        input.file_path,
        input.name,
    )?
    else {
        return Ok(());
    };
    let facade_use = status.use_syntax();
    let fix_support = internal_parent_facade_fix_support(&status, parent_facade_analysis);
    sink.findings.push(source::build_line_finding(
        ctx.source_cache,
        &status.parent_path,
        status.parent_line,
        FindingParams {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::InternalParentPubUseFacade,
            item: input
                .name
                .map(|name| StoredFinding::render_item(facade_use.unwrap_or("re-export"), name)),
            message: facade_use.map_or_else(
                || String::from("parent module re-export is acting as an internal facade"),
                |syntax| format!("parent module `{syntax}` is acting as an internal facade"),
            ),
            suggestion: None,
            fix_support,
            related,
            visibility_annotation: None,
            // `parent_facade_export_status` scans source text, so it cannot see
            // paths that exist only as tokens a proc macro in another crate emits.
            // Naming the item in `item_def_path` and the subtree in
            // `narrower_scope_def_path` hands the verdict to `caller_aware`, whose
            // HIR use sites are collected after macro expansion and do see them.
            item_def_path: Some(use_sites::def_path_string(ctx.tcx, input.def_id)),
            narrower_scope_def_path: Some(status.boundary_def_path.clone()),
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?);
    record_suspicious_pub_use_fact(ctx, input, Some(&status), fix_support, sink);
    Ok(())
}

/// Whether `--fix-pub-use` can delete this facade line. Removing it moves every
/// importer under the parent onto the declaring child module, so the fixer needs
/// two things the facade scan already decided: a boundary it can edit
/// (`ParentFacadeFixSupport::Supported` rules out globs and other re-export
/// spellings it cannot rewrite) and a chain that resolves to one declaring
/// module. Anything else stays a report-only finding.
fn internal_parent_facade_fix_support(
    status: &ParentFacadeExportStatus,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> FixSupport {
    let chain_resolved = matches!(
        parent_facade_analysis.map(|analysis| analysis.chain),
        Some(FacadeChainResolution::Resolved { .. })
    );
    if status.fix_support == ParentFacadeFixSupport::Supported && chain_resolved {
        FixSupport::PubUse
    } else {
        FixSupport::InternalParentFacade
    }
}

fn record_suspicious_pub_warning(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    warning_input: SuspiciousWarningInput<'_, '_, '_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let SuspiciousWarningInput {
        kind_label,
        annotation,
        parent_facade_analysis,
        signature_exposure,
        warning,
    } = warning_input;
    let SuspiciousWarning {
        fix_support,
        related,
        stale_parent_pub_use,
    } = warning;
    let bare_pub = matches!(annotation.syntax(), VisibilitySyntax::Public);
    // `ReexportIndex::has_public_reexport_outside_ancestors` finds a `pub use`
    // outside this item's ancestors that names it, and rustc requires a
    // re-exported declaration to be at least as visible as the re-export
    // (E0364) whether or not that re-export is reachable from outside. Only a
    // parent facade travels with the declaration when the fixers rewrite it, so
    // the finding is reported here without a fix.
    let public_reexport_constraint = if ctx.reexport_index.has_public_reexport_outside_ancestors(
        ctx.tcx,
        input.def_id,
        ctx.reexport_index.facade_subject(input.def_id),
    ) {
        PublicReexportConstraint::PinsDeclaration
    } else {
        PublicReexportConstraint::Absent
    };
    let details = suspicious_finding_details(
        ctx,
        input,
        SuspiciousFindingState {
            kind_label,
            annotation,
            parent_facade_analysis,
            signature_exposure,
            fallback_fix_support: fix_support,
            stale_parent_pub_use: stale_parent_pub_use.as_ref(),
            public_reexport_constraint,
        },
    );
    sink.findings.push(source::build_finding(
        ctx.tcx,
        input.file_path,
        input.highlight_span,
        FindingParams {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::SuspiciousPub,
            item: input
                .name
                .map(|name| StoredFinding::render_item(kind_label, name)),
            message: details.message,
            suggestion: Some(details.suggestion),
            fix_support: details.fix_support,
            related,
            visibility_annotation: Some(annotation.source().to_string()),
            item_def_path: details.item_def_path,
            narrower_scope_def_path: details.narrower_scope_def_path,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?);
    if bare_pub
        && stale_parent_pub_use.is_none()
        && public_reexport_constraint == PublicReexportConstraint::Absent
    {
        record_visibility_constraint(
            ctx,
            &ItemInfo {
                def_id:          input.def_id,
                file_path:       input.file_path,
                visibility_text: annotation.source(),
                kind_label:      input.kind_label,
                name:            input.name,
                highlight_span:  input.highlight_span,
                category:        input.category,
                facade_subject:  input.facade_subject,
            },
            annotation,
            VisibilityConstraintInput {
                diagnostic_code: DiagnosticCode::SuspiciousPub,
                parent_facade_analysis,
                signature_exposure,
                outcome: StoredConstraintOutcome::Finding,
            },
            sink,
        );
    }
    record_suspicious_pub_use_fact(
        ctx,
        input,
        stale_parent_pub_use.as_ref(),
        details.fix_support,
        sink,
    );
    Ok(())
}

struct SuspiciousFindingDetails {
    message:                 String,
    suggestion:              String,
    fix_support:             FixSupport,
    item_def_path:           Option<String>,
    narrower_scope_def_path: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PublicReexportConstraint {
    Absent,
    PinsDeclaration,
}

#[derive(Clone, Copy)]
struct SuspiciousFindingState<'borrow, 'syntax, 'facade> {
    kind_label:                 &'borrow str,
    annotation:                 &'borrow VisibilityAnnotation<'syntax>,
    parent_facade_analysis:     Option<&'borrow ParentFacadeAnalysis<'facade>>,
    signature_exposure:         SignatureExposure,
    fallback_fix_support:       FixSupport,
    stale_parent_pub_use:       Option<&'borrow ParentFacadeExportStatus>,
    public_reexport_constraint: PublicReexportConstraint,
}

fn suspicious_finding_details(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    state: SuspiciousFindingState<'_, '_, '_>,
) -> SuspiciousFindingDetails {
    let SuspiciousFindingState {
        kind_label,
        annotation,
        parent_facade_analysis,
        signature_exposure,
        fallback_fix_support,
        stale_parent_pub_use,
        public_reexport_constraint,
    } = state;
    let bare_pub = matches!(annotation.syntax(), VisibilitySyntax::Public);
    let advice = suspicious_pub_advice(
        ctx,
        input,
        kind_label,
        annotation,
        parent_facade_analysis,
        signature_exposure,
        stale_parent_pub_use,
    );
    let (message, suggestion, fix_support, item_def_path, narrower_scope_def_path) = match advice {
        SuspiciousPubAdvice::Narrowing { suggestion, target } => {
            // `NarrowingTarget::Exact` permits rewriting a bare `pub` whose
            // exact boundary resolved in place, but only when no facade line has
            // to move with it: a stale facade
            // makes the repair a multi-file edit this fixer does not perform.
            //
            // The rewrite spells `pub(in crate::<path>)`, so it is confined to
            // `PubInPath::Required`. Under `Forbidden` that annotation is what
            // `forbidden_pub_in_crate` reports as an error on the next run, and
            // under `Permitted` it is one accepted spelling among several
            // rather than the one the policy asks for.
            let fallback_support = || {
                if bare_pub {
                    facade_chain_fix_support(parent_facade_analysis, fallback_fix_support)
                } else {
                    FixSupport::None
                }
            };
            let fix_support = match public_reexport_constraint {
                PublicReexportConstraint::PinsDeclaration => FixSupport::None,
                PublicReexportConstraint::Absent => match &target {
                    NarrowingTarget::Exact(_) => {
                        if bare_pub
                            && stale_parent_pub_use.is_none()
                            && matches!(
                                ctx.settings.visibility_config.pub_in_path,
                                PubInPath::Required
                            )
                        {
                            FixSupport::RestrictedAnnotation
                        } else {
                            fallback_support()
                        }
                    },
                    NarrowingTarget::CallerSuppression {
                        enclosing_module: _,
                    } => fallback_support(),
                },
            };
            (
                policy::suspicious_pub_note(input.crate_kind, kind_label),
                suggestion,
                fix_support,
                Some(use_sites::def_path_string(ctx.tcx, input.def_id)),
                Some(target.into_scope_def_path()),
            )
        },
        SuspiciousPubAdvice::Structural {
            message,
            suggestion,
        } => (message, suggestion, FixSupport::None, None, None),
    };
    SuspiciousFindingDetails {
        message,
        suggestion,
        fix_support,
        item_def_path,
        narrower_scope_def_path,
    }
}

fn record_suspicious_pub_use_fact(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    stale_parent_pub_use: Option<&ParentFacadeExportStatus>,
    fix_support: FixSupport,
    sink: &mut FindingsSink,
) {
    let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.name) else {
        return;
    };
    if fix_support != FixSupport::PubUse
        || !matches!(input.visibility_syntax, VisibilitySyntax::Public)
    {
        return;
    }
    let child_line = ctx
        .tcx
        .sess
        .source_map()
        .lookup_char_pos(input.highlight_span.lo())
        .line;
    let Some(child_module) = input
        .file_path
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|stem| *stem != "mod")
        .map(String::from)
    else {
        return;
    };
    sink.pub_use_fix_facts.push(StoredPubUseFixFact {
        child_path: input.file_path.to_string_lossy().into_owned(),
        child_line,
        child_item_name: item_name.to_string(),
        parent_path: status.parent_path.to_string_lossy().into_owned(),
        parent_line: status.parent_line,
        child_module,
    });
}

enum FacadeCleanup<'analysis, 'facade> {
    NoStaleFacade,
    Blocked {
        analysis: &'analysis ParentFacadeAnalysis<'facade>,
    },
    Resolvable {
        retained_requirement: RetainedFacadeRequirement,
    },
}

impl<'analysis, 'facade> FacadeCleanup<'analysis, 'facade> {
    const fn classify(
        stale_parent_facade: Option<&ParentFacadeExportStatus>,
        parent_facade_analysis: Option<&'analysis ParentFacadeAnalysis<'facade>>,
    ) -> Self {
        match (stale_parent_facade, parent_facade_analysis) {
            (None, _) => Self::NoStaleFacade,
            (Some(_), Some(analysis)) => match analysis.chain {
                FacadeChainResolution::Unresolvable { .. } => Self::Blocked { analysis },
                FacadeChainResolution::Resolved { .. } => Self::Resolvable {
                    retained_requirement: analysis.retained_facade_requirement,
                },
            },
            (Some(_), None) => Self::Resolvable {
                retained_requirement: RetainedFacadeRequirement::Absent,
            },
        }
    }
}

fn suspicious_pub_advice(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    kind_label: &str,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: SignatureExposure,
    stale_parent_facade: Option<&ParentFacadeExportStatus>,
) -> SuspiciousPubAdvice {
    let parent_scope_def_path = use_sites::parent_module_def_path(ctx.tcx, input.def_id);
    match FacadeCleanup::classify(stale_parent_facade, parent_facade_analysis) {
        FacadeCleanup::NoStaleFacade => {},
        FacadeCleanup::Blocked { analysis } => {
            return structural_unresolvable_facade_cleanup_advice(ctx, kind_label, Some(analysis));
        },
        FacadeCleanup::Resolvable {
            retained_requirement,
        } => {
            let retained_facade_reach = match retained_requirement {
                RetainedFacadeRequirement::Absent => None,
                RetainedFacadeRequirement::Required(reach) => Some(reach),
            };
            let required_reach =
                joined_required_reach(ctx, retained_facade_reach, signature_exposure);
            if !matches!(annotation.syntax(), VisibilitySyntax::Public) && required_reach.is_none()
            {
                return SuspiciousPubAdvice::Narrowing {
                    suggestion: format!(
                        "remove the parent facade and the now-unneeded `{}` annotation",
                        annotation.display_source()
                    ),
                    target:     NarrowingTarget::CallerSuppression {
                        enclosing_module: parent_scope_def_path,
                    },
                };
            }
            let pub_super_reach = VisibilityAnnotation::Parent.reach(input.def_id, ctx.tcx);
            if let Some(required_reach) = required_reach
                && !matches!(
                    pub_super_reach.compare(required_reach, ctx.tcx),
                    Some(Ordering::Equal | Ordering::Greater)
                )
            {
                return structural_facade_cleanup_advice(
                    ctx,
                    kind_label,
                    required_reach,
                    input.def_id,
                );
            }
            return SuspiciousPubAdvice::Narrowing {
                suggestion: policy::consider_using("pub(super)"),
                target:     NarrowingTarget::CallerSuppression {
                    enclosing_module: parent_scope_def_path,
                },
            };
        },
    }
    if let Some(parent_facade_analysis) = parent_facade_analysis
        && let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain
    {
        let required = signature_exposure.combined_with(required, ctx);
        // `suspicious_pub_advice` derives `exact_boundary` from `required`:
        // only a `pub(in crate::<path>)` reach yields a boundary a fixer can
        // spell out. `pub` and `pub(crate)` fall back to the enclosing module,
        // which is a suppression key, not a rewrite target.
        let target = visibility_reach_boundary_path(required, ctx)
            .and_then(|path| ValidatedRestrictedBoundary::from_crate_rooted(&path))
            .map_or_else(
                || NarrowingTarget::CallerSuppression {
                    enclosing_module: parent_scope_def_path,
                },
                NarrowingTarget::Exact,
            );
        return SuspiciousPubAdvice::Narrowing {
            suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
            target,
        };
    }
    let SignatureExposure::ExposedAt(required) = signature_exposure else {
        return SuspiciousPubAdvice::Narrowing {
            suggestion: policy::consider_using("pub(super)"),
            target:     NarrowingTarget::CallerSuppression {
                enclosing_module: parent_scope_def_path,
            },
        };
    };
    let parent_reach = VisibilityAnnotation::Parent.reach(input.def_id, ctx.tcx);
    if matches!(
        parent_reach.compare(required, ctx.tcx),
        Some(Ordering::Equal | Ordering::Greater)
    ) {
        return SuspiciousPubAdvice::Narrowing {
            suggestion: policy::consider_using("pub(super)"),
            target:     NarrowingTarget::CallerSuppression {
                enclosing_module: parent_scope_def_path,
            },
        };
    }
    structural_signature_advice(ctx, kind_label, required, input.def_id)
}

fn structural_unresolvable_facade_cleanup_advice(
    ctx: &VisibilityContext<'_, '_>,
    kind_label: &str,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> SuspiciousPubAdvice {
    let blocker_advice = parent_facade_blocker_text(ctx, parent_facade_analysis)
        .unwrap_or_else(|| String::from("resolve the enclosing facade chain"));
    SuspiciousPubAdvice::Structural {
        message:    format!(
            "{kind_label} has a stale parent facade in a chain without an exact visibility boundary"
        ),
        suggestion: format!(
            "{blocker_advice}; then remove the stale parent facade and rerun `cargo mend`"
        ),
    }
}

fn structural_facade_cleanup_advice(
    ctx: &VisibilityContext<'_, '_>,
    kind_label: &str,
    required: VisibilityReach,
    def_id: LocalDefId,
) -> SuspiciousPubAdvice {
    let boundary_path = visibility_reach_boundary_path(required, ctx)
        .unwrap_or_else(|| String::from("crate-external"));
    SuspiciousPubAdvice::Structural {
        message:    format!(
            "{kind_label} must remain visible at `{boundary_path}` after removing the parent facade; \
             exact `pub(in ...)` visibility requires a resolved facade"
        ),
        suggestion: policy::no_facade_suggestion(
            NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required },
            &boundary_path,
            &use_sites::def_path_string(ctx.tcx, def_id),
        ),
    }
}

fn structural_signature_advice(
    ctx: &VisibilityContext<'_, '_>,
    kind_label: &str,
    required: VisibilityReach,
    def_id: LocalDefId,
) -> SuspiciousPubAdvice {
    let boundary_path = visibility_reach_boundary_path(required, ctx)
        .unwrap_or_else(|| String::from("crate-external"));
    SuspiciousPubAdvice::Structural {
        message:    format!(
            "{kind_label} is exposed through a signature at `{boundary_path}`; exact `pub(in ...)` \
             visibility requires a resolved facade"
        ),
        suggestion: policy::no_facade_suggestion(
            NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required },
            &boundary_path,
            &use_sites::def_path_string(ctx.tcx, def_id),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use rustc_middle::ty::Visibility;
    use rustc_span::DUMMY_SP;
    use rustc_span::def_id::CRATE_DEF_ID;
    use rustc_span::def_id::DefId;
    use rustc_span::def_id::DefIndex;
    use rustc_span::def_id::LOCAL_CRATE;

    use super::DisplayDefPath;
    use super::FacadeCleanup;
    use super::NarrowingTarget;
    use super::ParentScope;
    use super::PubCrateNarrowing;
    use super::ReachRequirements;
    use super::ValidatedRestrictedBoundary;
    use crate::compiler::facade::FacadeSpellingAgreement;
    use crate::compiler::facade::ParentFacadeExportStatus;
    use crate::compiler::facade::ParentFacadeFixSupport;
    use crate::compiler::facade::ParentFacadeReach;
    use crate::compiler::facade::ParentFacadeSpelling;
    use crate::compiler::facade::ParentFacadeUsage;
    use crate::compiler::facade::ParentReach;
    use crate::compiler::visibility::VisibilityAudience;
    use crate::compiler::visibility::annotation::VisibilityReach;
    use crate::compiler::visibility::annotation::VisibilitySyntax;
    use crate::compiler::visibility::policy::NoFacadeVisibilityRepair;
    use crate::compiler::visibility::scan::classify::ParentVisibility;
    use crate::compiler::visibility::use_sites::FacadeChainBlocker;
    use crate::compiler::visibility::use_sites::FacadeChainResolution;
    use crate::compiler::visibility::use_sites::FacadeUseKind;
    use crate::compiler::visibility::use_sites::ParentFacadeAnalysis;
    use crate::compiler::visibility::use_sites::ParentFacadeOccurrences;
    use crate::compiler::visibility::use_sites::ReexportOccurrence;
    use crate::compiler::visibility::use_sites::RetainedFacadeRequirement;

    #[test]
    fn reach_requirements_keep_facade_and_required_reach_together() {
        let facade = VisibilityReach::from(Visibility::Restricted(CRATE_DEF_ID.to_def_id()));
        let required = VisibilityReach::from(Visibility::<DefId>::Public);
        let requirements = ReachRequirements::Facade { facade, required };
        assert!(matches!(
            requirements.facade_reach().map(VisibilityReach::boundary),
            Some(crate::compiler::visibility::annotation::ReachBoundary::CrateRoot)
        ));
        assert!(
            requirements
                .required_reach()
                .is_some_and(|reach| reach.audience() == VisibilityAudience::Public)
        );
        let signature = ReachRequirements::SignatureOnly { required };
        assert!(signature.facade_reach().is_none());
        assert!(
            signature
                .required_reach()
                .is_some_and(|reach| reach.audience() == VisibilityAudience::Public)
        );
        assert!(ReachRequirements::Unconstrained.facade_reach().is_none());
        assert!(ReachRequirements::Unconstrained.required_reach().is_none());
    }

    #[test]
    fn parent_scope_preserves_root_and_enclosing_caller_advice() {
        let callers = BTreeSet::from([String::from("a::sibling")]);
        let root = ParentScope::for_module("", 0);
        assert!(matches!(root, ParentScope::CrateRoot));
        assert!(matches!(
            root.classify_callers("", &callers),
            NoFacadeVisibilityRepair::RemoveAnnotation
        ));
        assert_eq!(
            root.caller_note(NoFacadeVisibilityRepair::RemoveAnnotation, ""),
            None
        );
        let parent = ParentScope::for_module("a::child", 2);
        assert!(matches!(parent, ParentScope::Enclosing { def_path: "a" }));
        assert!(matches!(
            parent.classify_callers("a::child", &callers),
            NoFacadeVisibilityRepair::UseParentVisibility
        ));
        assert_eq!(
            parent.caller_note(NoFacadeVisibilityRepair::UseParentVisibility, "a::child"),
            Some(String::from(concat!(
                "no use of this item was found outside `crate::a`, ",
                "which `pub(super)` already reaches"
            )))
        );
        let shallow = ParentScope::for_module("a", 1);
        assert!(matches!(shallow, ParentScope::CrateRoot));
        assert!(matches!(
            shallow.classify_callers("a", &BTreeSet::from([String::from("sibling")])),
            NoFacadeVisibilityRepair::UseParentVisibility
        ));
        assert_eq!(
            shallow.caller_note(NoFacadeVisibilityRepair::UseParentVisibility, "a"),
            None
        );
    }

    #[test]
    fn display_def_paths_preserve_root_nested_and_import_names() {
        for (path, item_name, expected) in [
            ("Thing", Some("Alias"), "Thing"),
            ("a::Thing", Some("Alias"), "a::Thing"),
            ("{use#0}", Some("Alias"), "Alias"),
            ("a::{use#0}", Some("Alias"), "a::Alias"),
            ("{use#0}", None, "{use#0}"),
            ("a::{use#0}", None, "a::{use#0}"),
        ] {
            assert_eq!(
                DisplayDefPath::from(path).with_item_name(item_name),
                expected
            );
        }
    }

    #[test]
    fn narrowing_targets_require_a_checked_restricted_boundary() {
        for path in ["crate", "crate-external", "a::b", "crate::"] {
            assert!(ValidatedRestrictedBoundary::from_crate_rooted(path).is_none());
        }
        let exact = ValidatedRestrictedBoundary::from_crate_rooted("crate::a::b")
            .map(NarrowingTarget::Exact)
            .map(NarrowingTarget::into_scope_def_path);
        assert_eq!(exact.as_deref(), Some("a::b"));
        let suppression = NarrowingTarget::CallerSuppression {
            enclosing_module: String::from("a::child"),
        };
        assert_eq!(suppression.into_scope_def_path(), "a::child");
    }

    #[test]
    fn pub_crate_narrowing_classifies_depth_boundaries() {
        assert!(matches!(
            PubCrateNarrowing::classify(VisibilitySyntax::Public, ParentVisibility::Private, 0),
            PubCrateNarrowing::Ineligible
        ));
        assert!(matches!(
            PubCrateNarrowing::classify(VisibilitySyntax::Public, ParentVisibility::Private, 1),
            PubCrateNarrowing::Shallow
        ));
        assert!(matches!(
            PubCrateNarrowing::classify(VisibilitySyntax::Public, ParentVisibility::Private, 2),
            PubCrateNarrowing::Nested
        ));
        for depth in 0..=2 {
            assert!(matches!(
                PubCrateNarrowing::classify(
                    VisibilitySyntax::Crate,
                    ParentVisibility::Private,
                    depth
                ),
                PubCrateNarrowing::Ineligible
            ));
            assert!(matches!(
                PubCrateNarrowing::classify(
                    VisibilitySyntax::Public,
                    ParentVisibility::Public,
                    depth
                ),
                PubCrateNarrowing::Ineligible
            ));
        }
    }

    #[test]
    fn facade_cleanup_without_analysis_preserves_absent_requirement() {
        let stale = ParentFacadeExportStatus {
            usage:               ParentFacadeUsage::Unused,
            fix_support:         ParentFacadeFixSupport::Unsupported,
            parent_facade_reach: ParentFacadeReach {
                parent_reach:       ParentReach::ReachesParent,
                spelling:           ParentFacadeSpelling::Crate,
                spelling_agreement: FacadeSpellingAgreement::Consistent,
            },
            parent_path:         "src/lib.rs".into(),
            parent_rel_path:     "src/lib.rs".into(),
            parent_line:         1,
            boundary_def_path:   "crate".into(),
        };
        assert!(matches!(
            FacadeCleanup::classify(Some(&stale), None),
            FacadeCleanup::Resolvable {
                retained_requirement: RetainedFacadeRequirement::Absent,
            }
        ));
        let module_id = DefId {
            krate: LOCAL_CRATE,
            index: DefIndex::from_u32(1),
        };
        let visibility = Visibility::Restricted(module_id);
        let occurrence = ReexportOccurrence {
            use_def_id: CRATE_DEF_ID,
            owner_module: CRATE_DEF_ID,
            visibility,
            facade_spelling: ParentFacadeSpelling::Crate,
            spelling_agreement: FacadeSpellingAgreement::Consistent,
            use_kind: FacadeUseKind::Glob,
            alias: None,
            export_names: Vec::new(),
            span: DUMMY_SP,
            usage_by_name: Rc::default(),
        };
        let analysis = ParentFacadeAnalysis {
            nearest:                     ParentFacadeOccurrences {
                selected:           &occurrence,
                matching:           vec![&occurrence],
                spelling_agreement: FacadeSpellingAgreement::Consistent,
            },
            chain:                       FacadeChainResolution::Unresolvable {
                blocker: FacadeChainBlocker::Glob(&occurrence),
            },
            retained_facade_requirement: RetainedFacadeRequirement::Required(
                VisibilityReach::from(visibility),
            ),
        };
        assert!(matches!(
            FacadeCleanup::classify(Some(&stale), Some(&analysis)),
            FacadeCleanup::Blocked { analysis: blocked }
                if std::ptr::eq(blocked, &raw const analysis)
        ));
        assert!(matches!(
            FacadeCleanup::classify(None, Some(&analysis)),
            FacadeCleanup::NoStaleFacade
        ));
    }
}

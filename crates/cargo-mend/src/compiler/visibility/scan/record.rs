use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ffi::OsStr;

use anyhow::Result;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;

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
use crate::compiler::constants::PRELUDE_MODULE_NAME;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredCallerReconciliation;
use crate::compiler::persistence::StoredConstraintOutcome;
use crate::compiler::persistence::StoredExactBoundaryAcceptance;
use crate::compiler::persistence::StoredFacadeConstraint;
use crate::compiler::persistence::StoredFinding;
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
use crate::compiler::visibility::policy::ForbiddenPubCrateSuggestionReason;
use crate::compiler::visibility::policy::NoFacadeVisibilityRepair;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::compiler::visibility::use_sites::FacadeChainBlocker;
use crate::compiler::visibility::use_sites::FacadeChainResolution;
use crate::compiler::visibility::use_sites::ParentFacadeAnalysis;
use crate::compiler::visibility::use_sites::RetainedFacadeRequirement;
use crate::config::DiagnosticCode;
use crate::config::PreludePubMod;
use crate::config::PubInPath;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

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
    let signature_visibility_requirement = SignatureVisibilityRequirement::from(signature_exposure);

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

    let should_consider_narrow_to_pub_crate =
        matches!(annotation.syntax(), VisibilitySyntax::Public)
            && finding_context.parent_visibility == ParentVisibility::Private
            && policy::narrow_to_pub_crate_by_policy(finding_context.crate_kind);

    if should_consider_narrow_to_pub_crate
        && finding_context.logical_module_depth == 1
        && policy::allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        )
    {
        maybe_record_narrow_to_pub_crate(ctx, item, signature_visibility_requirement, sink)?;
    }

    if should_consider_narrow_to_pub_crate && finding_context.logical_module_depth > 1 {
        maybe_record_narrow_to_pub_crate_nested(
            ctx,
            item,
            parent_facade_analysis.as_ref(),
            signature_visibility_requirement,
            sink,
        )?;
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
    signature_exposure: Option<VisibilityReach>,
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
    let annotation_reaches_signature = signature_exposure.is_some_and(|required| {
        matches!(
            annotation
                .reach(item.def_id, ctx.tcx)
                .compare(required, ctx.tcx),
            Some(Ordering::Equal | Ordering::Greater)
        )
    });
    if parent_facade_exports_item(parent_facade_analysis)
        || facade::path_exists_outside_child_module(
            ctx.source_cache,
            ctx.source_root,
            ctx.tcx,
            ctx.module_sources,
            &use_sites::parent_module_path_segments(ctx.tcx, item.def_id),
            name,
        )
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
    signature_exposure: Option<VisibilityReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    match annotation.syntax() {
        VisibilitySyntax::Crate | VisibilitySyntax::InCrate => record_forbidden_pub_crate(
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
    signature_exposure:     Option<VisibilityReach>,
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
    let signature_requirement =
        signature_exposure.and_then(|reach| stored_visibility_reach(reach, ctx));
    let facade = match parent_facade_analysis.map(|analysis| analysis.chain) {
        Some(FacadeChainResolution::Resolved { required }) => {
            let Some(required) = stored_visibility_reach(required, ctx) else {
                return;
            };
            StoredFacadeConstraint::Resolved { required }
        },
        Some(FacadeChainResolution::Unresolvable { .. }) => StoredFacadeConstraint::Blocked,
        None => StoredFacadeConstraint::Absent,
    };
    let exact_boundary_acceptance =
        exact_boundary_acceptance(ctx, item, annotation, diagnostic_code);
    let pub_in_reconciles_callers = diagnostic_code == DiagnosticCode::ForbiddenPubInCrate
        && matches!(annotation.syntax(), VisibilitySyntax::InPath(_));
    let pub_crate_reconciles_callers =
        diagnostic_code == DiagnosticCode::ForbiddenPubCrate && signature_requirement.is_some();
    let caller_reconciliation = if item.category == ItemCategory::Declaration
        && parent_facade_analysis.is_none()
        && (pub_in_reconciles_callers || pub_crate_reconciles_callers)
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
            facade,
            exact_boundary_acceptance,
            caller_reconciliation,
            outcome,
        });
}

fn exact_boundary_acceptance(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    diagnostic_code: DiagnosticCode,
) -> StoredExactBoundaryAcceptance {
    let eligible = match diagnostic_code {
        DiagnosticCode::ForbiddenPubCrate => {
            matches!(annotation.syntax(), VisibilitySyntax::Crate)
        },
        DiagnosticCode::ForbiddenPubInCrate => {
            item.category == ItemCategory::Declaration
                && matches!(
                    annotation.syntax(),
                    VisibilitySyntax::InPath(PathSpelling::CrateRooted)
                )
                && matches!(
                    ctx.settings.visibility_config.pub_in_path,
                    PubInPath::Permitted | PubInPath::Required
                )
        },
        DiagnosticCode::ReviewPubMod
        | DiagnosticCode::SuspiciousPub
        | DiagnosticCode::UnusedPub
        | DiagnosticCode::PreferModuleImport
        | DiagnosticCode::InlinePathQualifiedType
        | DiagnosticCode::ShortenLocalCrateImport
        | DiagnosticCode::ReplaceDeepSuperImport
        | DiagnosticCode::WildcardParentPubUse
        | DiagnosticCode::InternalParentPubUseFacade
        | DiagnosticCode::NarrowToPubCrate
        | DiagnosticCode::FieldVisibilityWiderThanType
        | DiagnosticCode::ImportsAtTop => false,
    };
    if eligible {
        StoredExactBoundaryAcceptance::Eligible
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

fn record_forbidden_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: Option<VisibilityReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let policy_permits_pub_crate = policy::allow_pub_crate_by_policy(
        finding_context.crate_kind,
        finding_context.module_location,
        finding_context.parent_visibility,
    );
    let resolved_chain_reach = resolved_facade_reach(parent_facade_analysis);
    let required_reach = joined_required_reach(ctx, resolved_chain_reach, signature_exposure);
    let annotation_reach = annotation.reach(item.def_id, ctx.tcx);
    let pub_crate_is_permitted = resolved_chain_reach.map_or_else(
        || {
            policy_permits_pub_crate
                && signature_exposure.is_none_or(|required| {
                    matches!(
                        annotation_reach.compare(required, ctx.tcx),
                        Some(Ordering::Equal | Ordering::Greater)
                    )
                })
        },
        |_| {
            required_reach.is_some_and(|required| {
                annotation_reach.compare(required, ctx.tcx) == Some(Ordering::Equal)
            })
        },
    );
    if matches!(annotation.syntax(), VisibilitySyntax::Crate) && pub_crate_is_permitted {
        record_visibility_constraint(
            ctx,
            item,
            annotation,
            VisibilityConstraintInput {
                diagnostic_code: DiagnosticCode::ForbiddenPubCrate,
                parent_facade_analysis,
                signature_exposure,
                outcome: StoredConstraintOutcome::Accepted,
            },
            sink,
        );
        return Ok(false);
    }

    let exact_restricted_boundary =
        resolved_chain_reach
            .zip(required_reach)
            .and_then(|(_, required)| {
                (!reach_is_pub_crate(required, ctx) && required.to_source(ctx.tcx) != "pub")
                    .then_some(required)
            });
    let repair_context = match exact_restricted_boundary {
        Some(required) => ForbiddenPubCrateRepairContext::ExactRestrictedBoundary { required },
        None if matches!(annotation.syntax(), VisibilitySyntax::InCrate)
            && pub_crate_is_permitted =>
        {
            ForbiddenPubCrateRepairContext::CanonicalPubCrate
        },
        None => ForbiddenPubCrateRepairContext::PolicyFallback,
    };
    let advice = forbidden_pub_crate_advice(
        ctx,
        item,
        annotation,
        ForbiddenPubCrateAdviceInput {
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
        FindingParams {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ForbiddenPubCrate,
            item:                    None,
            message:                 advice.message,
            suggestion:              Some(advice.suggestion),
            fix_support:             FixSupport::None,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?;
    sink.findings.push(finding);
    record_visibility_constraint(
        ctx,
        item,
        annotation,
        VisibilityConstraintInput {
            diagnostic_code: DiagnosticCode::ForbiddenPubCrate,
            parent_facade_analysis,
            signature_exposure,
            outcome: StoredConstraintOutcome::Finding,
        },
        sink,
    );
    Ok(true)
}

struct ForbiddenPubCrateAdvice {
    message:    String,
    suggestion: String,
}

struct ForbiddenPubCrateNoFacadeAdvice {
    suggestion_reason: ForbiddenPubCrateSuggestionReason,
}

#[derive(Clone, Copy)]
enum ForbiddenPubCrateRepairContext {
    ExactRestrictedBoundary { required: VisibilityReach },
    CanonicalPubCrate,
    PolicyFallback,
}

#[derive(Clone, Copy)]
struct ForbiddenPubCrateAdviceInput<'borrow, 'facade> {
    finding_context:        &'borrow VisibilityFindingContext,
    parent_facade_analysis: Option<&'borrow ParentFacadeAnalysis<'facade>>,
    signature_exposure:     Option<VisibilityReach>,
    repair_context:         ForbiddenPubCrateRepairContext,
    sink:                   &'borrow FindingsSink,
}

fn forbidden_pub_crate_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    input: ForbiddenPubCrateAdviceInput<'_, '_>,
) -> ForbiddenPubCrateAdvice {
    let ForbiddenPubCrateAdviceInput {
        finding_context,
        parent_facade_analysis,
        signature_exposure,
        repair_context,
        sink,
    } = input;
    if signature_exposure.is_some_and(VisibilityReach::is_public) {
        let suggestion_reason = ForbiddenPubCrateSuggestionReason::PublicSignatureExposure;
        let generic_message = format!(
            "use of `{}` is forbidden by policy",
            annotation.display_source()
        );
        return ForbiddenPubCrateAdvice {
            message:    suggestion_reason.headline(generic_message),
            suggestion: policy::forbidden_pub_crate_suggestion(suggestion_reason),
        };
    }

    let fallback = || match repair_context {
        ForbiddenPubCrateRepairContext::ExactRestrictedBoundary { required } => {
            let message = match annotation.syntax() {
                VisibilitySyntax::Crate => {
                    String::from("use of `pub(crate)` does not match the parent facade boundary")
                },
                VisibilitySyntax::InCrate => {
                    String::from("`pub(in crate)` is wider than the exact parent facade boundary")
                },
                _ => format!(
                    "use of `{}` is forbidden by policy",
                    annotation.display_source()
                ),
            };
            ForbiddenPubCrateAdvice {
                message,
                suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
            }
        },
        ForbiddenPubCrateRepairContext::CanonicalPubCrate => ForbiddenPubCrateAdvice {
            message:    String::from("`pub(in crate)` is a redundant spelling of `pub(crate)`"),
            suggestion: policy::consider_using("pub(crate)"),
        },
        ForbiddenPubCrateRepairContext::PolicyFallback => {
            let no_facade_advice = match (signature_exposure, parent_facade_analysis) {
                (_, Some(parent_facade_analysis)) => ForbiddenPubCrateNoFacadeAdvice {
                    suggestion_reason: forbidden_pub_crate_parent_facade_reason(
                        ctx,
                        parent_facade_analysis,
                        finding_context.module_location,
                    ),
                },
                (signature_exposure, None) => forbidden_pub_crate_no_facade_advice(
                    ctx,
                    item,
                    signature_exposure,
                    finding_context.module_location,
                    sink,
                ),
            };
            let generic_message = format!(
                "use of `{}` is forbidden by policy",
                annotation.display_source()
            );
            let message = no_facade_advice.suggestion_reason.headline(generic_message);
            ForbiddenPubCrateAdvice {
                message,
                suggestion: policy::forbidden_pub_crate_suggestion(
                    no_facade_advice.suggestion_reason,
                ),
            }
        },
    };
    parent_facade_blocker_text(ctx, parent_facade_analysis).map_or_else(fallback, |suggestion| {
        ForbiddenPubCrateAdvice {
            message: format!(
                "use of `{}` is forbidden by policy",
                annotation.display_source()
            ),
            suggestion,
        }
    })
}

fn forbidden_pub_crate_parent_facade_reason(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: &ParentFacadeAnalysis<'_>,
    module_location: ModuleLocation,
) -> ForbiddenPubCrateSuggestionReason {
    let parent_facade_reach = parent_facade_reach(parent_facade_analysis, ctx);
    if !parent_facade_reach.reaches_parent {
        return ForbiddenPubCrateSuggestionReason::LocationPolicy { module_location };
    }
    let boundary_path = resolved_facade_boundary_path(parent_facade_analysis, ctx);
    match (
        parent_facade_reach.spelling,
        parent_facade_reach.spelling_conflict,
        boundary_path,
    ) {
        (ParentFacadeSpelling::Super, false, Some(boundary_path)) => {
            ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacade { boundary_path }
        },
        (ParentFacadeSpelling::Super, false, None) => {
            ForbiddenPubCrateSuggestionReason::ExactPubSuperParentFacadeWithoutKnownBoundary
        },
        (_, _, Some(boundary_path)) => {
            ForbiddenPubCrateSuggestionReason::ParentFacade { boundary_path }
        },
        (_, _, None) => ForbiddenPubCrateSuggestionReason::ParentFacadeWithoutKnownBoundary,
    }
}

fn forbidden_pub_crate_no_facade_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    signature_exposure: Option<VisibilityReach>,
    module_location: ModuleLocation,
    sink: &FindingsSink,
) -> ForbiddenPubCrateNoFacadeAdvice {
    let Some(signature_exposure) = signature_exposure else {
        return ForbiddenPubCrateNoFacadeAdvice {
            suggestion_reason: ForbiddenPubCrateSuggestionReason::LocationPolicy {
                module_location,
            },
        };
    };
    let Some(signature_boundary_path) = visibility_reach_boundary_path(signature_exposure, ctx)
    else {
        return ForbiddenPubCrateNoFacadeAdvice {
            suggestion_reason: ForbiddenPubCrateSuggestionReason::LocationPolicy {
                module_location,
            },
        };
    };
    let item_def_path = use_sites::def_path_string(ctx.tcx, item.def_id);
    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    let callers = current_pass_callers(sink, &item_def_path);
    let caller_repair = policy::classify_no_facade_callers(
        &item_module,
        policy::parent_scope_def_path(&item_module),
        &callers,
    );
    let repair =
        merge_no_facade_signature_reach(ctx, item, caller_repair, Some(signature_exposure));
    let boundary_path = match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            visibility_reach_boundary_path(required, ctx).unwrap_or(signature_boundary_path)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility
        | NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => {
            signature_boundary_path
        },
    };
    ForbiddenPubCrateNoFacadeAdvice {
        suggestion_reason: ForbiddenPubCrateSuggestionReason::NoFacadeRepair {
            boundary_path,
            repair,
        },
    }
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: Option<VisibilityReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let resolved_chain_reach = resolved_facade_reach(parent_facade_analysis);
    let required_reach = joined_required_reach(ctx, resolved_chain_reach, signature_exposure);
    if exact_pub_in_boundary_is_allowed(ctx, item, annotation, resolved_chain_reach, required_reach)
    {
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
    let (message, suggestion) = forbidden_pub_in_advice(
        ctx,
        item,
        annotation,
        finding_context,
        parent_facade_analysis,
        signature_exposure,
        sink,
    )
    .into_message_and_suggestion();
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
            related: None,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
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
    let suggestion_reason = ForbiddenPubCrateSuggestionReason::PublicSignatureExposure;
    let generic_message = format!(
        "use of `{}` outside an exact facade boundary is forbidden by policy",
        annotation.display_source()
    );
    ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
        message:    suggestion_reason.headline(generic_message),
        suggestion: policy::forbidden_pub_crate_suggestion(suggestion_reason),
    }
}

#[derive(Clone, Copy)]
enum CanonicalPubInSpelling {
    Parent,
    Current,
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
}

impl ForbiddenPubInAdvice {
    fn into_message_and_suggestion(self) -> (String, Option<String>) {
        match self {
            Self::NoSuggestion { message } => (message, None),
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
            } => (message, Some(suggestion)),
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
        }
    }
}

fn forbidden_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: Option<VisibilityReach>,
    sink: &FindingsSink,
) -> ForbiddenPubInAdvice {
    let annotation_reach = annotation.reach(item.def_id, ctx.tcx);
    let public_signature_bypasses_blocker = signature_exposure
        .is_some_and(VisibilityReach::is_public)
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
            SignatureVisibilityRequirement::from(signature_exposure),
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

fn exact_pub_in_boundary_is_allowed(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    resolved_facade_reach: Option<VisibilityReach>,
    required_reach: Option<VisibilityReach>,
) -> bool {
    matches!(
        ctx.settings.visibility_config.pub_in_path,
        PubInPath::Permitted | PubInPath::Required
    ) && item.category == ItemCategory::Declaration
        && resolved_facade_reach.is_some()
        && matches!(
            annotation.syntax(),
            VisibilitySyntax::InPath(PathSpelling::CrateRooted)
        )
        && required_reach.is_some_and(|required| {
            annotation
                .reach(item.def_id, ctx.tcx)
                .compare(required, ctx.tcx)
                == Some(Ordering::Equal)
        })
}

fn joined_required_reach(
    ctx: &VisibilityContext<'_, '_>,
    facade_reach: Option<VisibilityReach>,
    signature_exposure: Option<VisibilityReach>,
) -> Option<VisibilityReach> {
    match (facade_reach, signature_exposure) {
        (Some(facade_reach), Some(signature_exposure)) => {
            Some(facade_reach.join(signature_exposure, ctx.tcx))
        },
        (Some(facade_reach), None) => Some(facade_reach),
        (None, Some(signature_exposure)) => Some(signature_exposure),
        (None, None) => None,
    }
}

fn merge_no_facade_signature_reach(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    caller_repair: NoFacadeVisibilityRepair,
    signature_exposure: Option<VisibilityReach>,
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
                Some(required_reach)
                    if !matches!(
                        VisibilityAnnotation::Parent
                            .reach(item.def_id, ctx.tcx)
                            .compare(required_reach, ctx.tcx),
                        Some(Ordering::Equal | Ordering::Greater)
                    ) =>
                {
                    NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach {
                        required: required_reach,
                    }
                },
                Some(_) | None => NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations,
            };
            return repair;
        },
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            return NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required };
        },
    };
    let required_reach = signature_exposure.map_or(caller_reach, |signature_reach| {
        caller_reach.join(signature_reach, ctx.tcx)
    });
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
    signature_exposure: Option<VisibilityReach>,
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
    signature_exposure: Option<VisibilityReach>,
) -> ForbiddenPubInAdvice {
    let (original_source, canonical_annotation) = match spelling {
        CanonicalPubInSpelling::Parent => ("pub(in super)", VisibilityAnnotation::Parent),
        CanonicalPubInSpelling::Current => ("pub(in self)", VisibilityAnnotation::Current),
    };
    let canonical_reach = canonical_annotation.reach(item.def_id, ctx.tcx);
    if let Some(required) = signature_exposure
        && !matches!(
            canonical_reach.compare(required, ctx.tcx),
            Some(Ordering::Equal | Ordering::Greater)
        )
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
            suggestion: policy::no_facade_suggestion(repair, &boundary_path),
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

fn resolved_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    facade_reach: VisibilityReach,
    signature_visibility_requirement: SignatureVisibilityRequirement,
) -> ForbiddenPubInAdvice {
    let required = signature_visibility_requirement.combined_with(facade_reach, ctx);
    let comparison = annotation_reach.compare(required, ctx.tcx);
    if comparison == Some(Ordering::Equal) && reach_is_pub_crate(required, ctx) {
        return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    String::from("parent facade caps reach at `pub(crate)`"),
            suggestion: policy::consider_using("pub(crate)"),
        };
    }
    if matches!(
        annotation.syntax(),
        VisibilitySyntax::InPath(PathSpelling::CrateRooted)
    ) && comparison == Some(Ordering::Equal)
        && item.category == ItemCategory::Declaration
        && matches!(
            ctx.settings.visibility_config.pub_in_path,
            PubInPath::Forbidden
        )
    {
        return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    format!(
                "use of `{}` is disabled by project visibility policy",
                annotation.display_source()
            ),
            suggestion: format!(
                "{}; or set `pub_in_path = \"permitted\"`",
                policy::consider_using("pub")
            ),
        };
    }
    if comparison == Some(Ordering::Equal) {
        exact_restricted_spelling_advice(ctx, annotation, required)
    } else {
        let message = if annotation_reach.compare(facade_reach, ctx.tcx) == Some(Ordering::Equal) {
            format!(
                "signature exposure requires the wider `{}` annotation",
                required.to_source(ctx.tcx)
            )
        } else if comparison == Some(Ordering::Greater) {
            format!(
                "`{}` is wider than the exact parent facade boundary",
                annotation.display_source()
            )
        } else {
            format!(
                "use of `{}` does not match the parent facade boundary",
                annotation.display_source()
            )
        };
        ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message,
            suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
        }
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
    signature_exposure: Option<VisibilityReach>,
    sink: &FindingsSink,
) -> ForbiddenPubInAdvice {
    if signature_exposure.is_some_and(VisibilityReach::is_public) {
        let suggestion_reason = ForbiddenPubCrateSuggestionReason::PublicSignatureExposure;
        let generic_message = format!(
            "use of `{}` outside an exact facade boundary is forbidden by policy",
            annotation.display_source()
        );
        return ForbiddenPubInAdvice::SuggestionWithoutCallerRefinement {
            message:    suggestion_reason.headline(generic_message),
            suggestion: policy::forbidden_pub_crate_suggestion(suggestion_reason),
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

    if item.category == ItemCategory::Use {
        // Resolved paths name the imported target, not the local `use` item,
        // so an empty caller set cannot prove that its alias has no users.
        return ForbiddenPubInAdvice::NoSuggestion {
            message: format!(
                "use of `{}` on a `use` item is forbidden by policy",
                annotation.display_source()
            ),
        };
    }

    let item_def_path = use_sites::def_path_string(ctx.tcx, item.def_id);
    let item_module = use_sites::parent_module_def_path(ctx.tcx, item.def_id);
    let parent_scope = if finding_context.logical_module_depth == 0 {
        ""
    } else {
        policy::parent_scope_def_path(&item_module)
    };
    let boundary_path = policy::canonical_pub_in_boundary(&item_module, annotation.source())
        .or_else(|| visibility_reach_boundary_path(annotation_reach, ctx))
        .unwrap_or_else(|| String::from("crate"));
    let callers = current_pass_callers(sink, &item_def_path);
    let caller_repair = policy::classify_no_facade_callers(&item_module, parent_scope, &callers);
    let repair = merge_no_facade_signature_reach(ctx, item, caller_repair, signature_exposure);
    let boundary_path = match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { required } => {
            visibility_reach_boundary_path(required, ctx).unwrap_or(boundary_path)
        },
        NoFacadeVisibilityRepair::RemoveAnnotation
        | NoFacadeVisibilityRepair::UseParentVisibility
        | NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations => boundary_path,
    };
    let message = policy::no_facade_headline(
        repair,
        format!(
            "use of `{}` outside an exact facade boundary is forbidden by policy",
            annotation.display_source()
        ),
    );
    let suggestion = policy::no_facade_suggestion(repair, &boundary_path);
    match repair {
        NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { .. } => {
            ForbiddenPubInAdvice::StructuralSuggestionControlledBySignatureReach {
                message,
                suggestion,
            }
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
        },
    }
}

fn current_pass_callers(sink: &FindingsSink, item_def_path: &str) -> BTreeSet<String> {
    sink.use_sites
        .callers(item_def_path)
        .cloned()
        .unwrap_or_default()
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
    // A crate-root `pub mod prelude;` follows the resolved `prelude_pub_mod` setting.
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
        },
    )?);
    Ok(())
}

fn maybe_record_narrow_to_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    signature_visibility_requirement: SignatureVisibilityRequirement,
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
    if !signature_visibility_requirement.is_satisfied_by(crate_reach, ctx) {
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
    signature_visibility_requirement: SignatureVisibilityRequirement,
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
    let combined_required_reach = signature_visibility_requirement.combined_with(required, ctx);
    let crate_reach = VisibilityAnnotation::Crate.reach(item.def_id, ctx.tcx);
    if parent_facade_analysis.nearest.spelling_conflict
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
        parent_facade_analysis.nearest.spelling_conflict,
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

#[derive(Clone, Copy)]
enum SignatureVisibilityRequirement {
    Absent,
    Required(VisibilityReach),
}

impl From<Option<VisibilityReach>> for SignatureVisibilityRequirement {
    fn from(signature_exposure: Option<VisibilityReach>) -> Self {
        signature_exposure.map_or(Self::Absent, Self::Required)
    }
}

impl SignatureVisibilityRequirement {
    fn is_satisfied_by(self, candidate: VisibilityReach, ctx: &VisibilityContext<'_, '_>) -> bool {
        match self {
            Self::Absent => true,
            Self::Required(required) => matches!(
                candidate.compare(required, ctx.tcx),
                Some(Ordering::Equal | Ordering::Greater)
            ),
        }
    }

    fn combined_with(
        self,
        required: VisibilityReach,
        ctx: &VisibilityContext<'_, '_>,
    ) -> VisibilityReach {
        match self {
            Self::Absent => required,
            Self::Required(signature_reach) => required.join(signature_reach, ctx.tcx),
        }
    }
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
    signature_exposure: Option<VisibilityReach>,
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
    signature_exposure:     Option<VisibilityReach>,
    warning:                SuspiciousWarning,
}

/// Whether the narrowing advice resolved an exact module boundary that `--fix`
/// can write in place of a bare `pub`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NarrowingReplacement {
    /// `narrower_scope_def_path` is the exact boundary the item needs, so
    /// `pub(in crate::<path>)` is a correct rewrite.
    ExactBoundary,
    /// The advice names no writable annotation — `narrower_scope_def_path` is
    /// the enclosing module recorded for caller-aware suppression, not a
    /// boundary to write.
    Unavailable,
}

enum SuspiciousPubAdvice {
    Narrowing {
        suggestion:              String,
        narrower_scope_def_path: String,
        replacement:             NarrowingReplacement,
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
            fix_support: FixSupport::InternalParentFacade,
            related,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
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
    let advice = suspicious_pub_advice(
        ctx,
        input,
        kind_label,
        annotation,
        parent_facade_analysis,
        signature_exposure,
        stale_parent_pub_use.as_ref(),
    );
    let bare_pub = matches!(annotation.syntax(), VisibilitySyntax::Public);
    let (message, suggestion, fix_support, item_def_path, narrower_scope_def_path) = match advice {
        SuspiciousPubAdvice::Narrowing {
            suggestion,
            narrower_scope_def_path,
            replacement,
        } => {
            // A bare `pub` whose exact boundary resolved is rewritten in place,
            // but only when no facade line has to move with it: a stale facade
            // makes the repair a multi-file edit this fixer does not perform.
            //
            // The rewrite spells `pub(in crate::<path>)`, so it is confined to
            // `PubInPath::Required`. Under `Forbidden` that annotation is what
            // `forbidden_pub_in_crate` reports as an error on the next run, and
            // under `Permitted` it is one accepted spelling among several
            // rather than the one the policy asks for.
            let rewrites_annotation_only = bare_pub
                && replacement == NarrowingReplacement::ExactBoundary
                && stale_parent_pub_use.is_none()
                && matches!(
                    ctx.settings.visibility_config.pub_in_path,
                    PubInPath::Required
                );
            let fix_support = if rewrites_annotation_only {
                FixSupport::RestrictedAnnotation
            } else if bare_pub {
                facade_chain_fix_support(parent_facade_analysis, fix_support)
            } else {
                FixSupport::None
            };
            (
                policy::suspicious_pub_note(input.crate_kind, kind_label),
                suggestion,
                fix_support,
                Some(use_sites::def_path_string(ctx.tcx, input.def_id)),
                Some(narrower_scope_def_path),
            )
        },
        SuspiciousPubAdvice::Structural {
            message,
            suggestion,
        } => (message, suggestion, FixSupport::None, None, None),
    };
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
            message,
            suggestion: Some(suggestion),
            fix_support,
            related,
            visibility_annotation: Some(annotation.source().to_string()),
            item_def_path,
            narrower_scope_def_path,
        },
    )?);
    record_suspicious_pub_use_fact(ctx, input, stale_parent_pub_use.as_ref(), fix_support, sink);
    Ok(())
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

fn suspicious_pub_advice(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    kind_label: &str,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    signature_exposure: Option<VisibilityReach>,
    stale_parent_facade: Option<&ParentFacadeExportStatus>,
) -> SuspiciousPubAdvice {
    let parent_scope_def_path = use_sites::parent_module_def_path(ctx.tcx, input.def_id);
    if stale_parent_facade.is_some()
        && parent_facade_analysis.is_some_and(|analysis| {
            matches!(analysis.chain, FacadeChainResolution::Unresolvable { .. })
        })
    {
        return structural_unresolvable_facade_cleanup_advice(
            ctx,
            kind_label,
            parent_facade_analysis,
        );
    }
    if stale_parent_facade.is_some() {
        let retained_facade_reach = parent_facade_analysis.and_then(|analysis| {
            match analysis.retained_facade_requirement {
                RetainedFacadeRequirement::Absent => None,
                RetainedFacadeRequirement::Required(reach) => Some(reach),
            }
        });
        let required_reach = joined_required_reach(ctx, retained_facade_reach, signature_exposure);
        if !matches!(annotation.syntax(), VisibilitySyntax::Public) && required_reach.is_none() {
            return SuspiciousPubAdvice::Narrowing {
                suggestion:              format!(
                    "remove the parent facade and the now-unneeded `{}` annotation",
                    annotation.display_source()
                ),
                narrower_scope_def_path: parent_scope_def_path,
                replacement:             NarrowingReplacement::Unavailable,
            };
        }
        let pub_super_reach = VisibilityAnnotation::Parent.reach(input.def_id, ctx.tcx);
        if let Some(required_reach) = required_reach
            && !matches!(
                pub_super_reach.compare(required_reach, ctx.tcx),
                Some(Ordering::Equal | Ordering::Greater)
            )
        {
            return structural_facade_cleanup_advice(ctx, kind_label, required_reach);
        }
        return SuspiciousPubAdvice::Narrowing {
            suggestion:              policy::consider_using("pub(super)"),
            narrower_scope_def_path: parent_scope_def_path,
            replacement:             NarrowingReplacement::Unavailable,
        };
    }
    if let Some(parent_facade_analysis) = parent_facade_analysis
        && let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain
    {
        let required = signature_exposure.map_or(required, |exposure_reach| {
            required.join(exposure_reach, ctx.tcx)
        });
        // Only a `pub(in crate::<path>)` reach yields a boundary a fixer can
        // spell out. `pub` and `pub(crate)` fall back to the enclosing module,
        // which is a suppression key, not a rewrite target.
        let exact_boundary = visibility_reach_boundary_path(required, ctx)
            .and_then(|path| path.strip_prefix("crate::").map(String::from));
        let (narrower_scope_def_path, replacement) = exact_boundary.map_or_else(
            || {
                (
                    parent_scope_def_path.clone(),
                    NarrowingReplacement::Unavailable,
                )
            },
            |boundary| (boundary, NarrowingReplacement::ExactBoundary),
        );
        return SuspiciousPubAdvice::Narrowing {
            suggestion: policy::consider_using(&required.to_source(ctx.tcx)),
            narrower_scope_def_path,
            replacement,
        };
    }
    let Some(required) = signature_exposure else {
        return SuspiciousPubAdvice::Narrowing {
            suggestion:              policy::consider_using("pub(super)"),
            narrower_scope_def_path: parent_scope_def_path,
            replacement:             NarrowingReplacement::Unavailable,
        };
    };
    let parent_reach = VisibilityAnnotation::Parent.reach(input.def_id, ctx.tcx);
    if matches!(
        parent_reach.compare(required, ctx.tcx),
        Some(Ordering::Equal | Ordering::Greater)
    ) {
        return SuspiciousPubAdvice::Narrowing {
            suggestion:              policy::consider_using("pub(super)"),
            narrower_scope_def_path: parent_scope_def_path,
            replacement:             NarrowingReplacement::Unavailable,
        };
    }
    structural_signature_advice(ctx, kind_label, required)
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
        ),
    }
}

fn structural_signature_advice(
    ctx: &VisibilityContext<'_, '_>,
    kind_label: &str,
    required: VisibilityReach,
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
        ),
    }
}

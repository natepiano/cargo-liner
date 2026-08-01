use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ffi::OsStr;

use anyhow::Result;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;

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
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredPubUseFixFact;
use crate::compiler::visibility::annotation::PathSpelling;
use crate::compiler::visibility::annotation::VisibilityAnnotation;
use crate::compiler::visibility::annotation::VisibilityReach;
use crate::compiler::visibility::annotation::VisibilitySyntax;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::compiler::visibility::use_sites::FacadeChainBlocker;
use crate::compiler::visibility::use_sites::FacadeChainResolution;
use crate::compiler::visibility::use_sites::ParentFacadeAnalysis;
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

    if record_forbidden_visibility_annotation(
        ctx,
        item,
        &annotation,
        &finding_context,
        parent_facade_analysis.as_ref(),
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
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if should_consider_narrow_to_pub_crate && finding_context.logical_module_depth > 1 {
        maybe_record_narrow_to_pub_crate_nested(ctx, item, parent_facade_analysis.as_ref(), sink)?;
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
    if parent_facade_exports_item(parent_facade_analysis)
        || facade::path_exists_outside_child_module(
            ctx.source_cache,
            ctx.source_root,
            ctx.tcx,
            ctx.module_sources,
            &use_sites::parent_module_path_segments(ctx.tcx, item.def_id),
            name,
        )
        || policy::has_signature_exposure_allowance(ctx, item.def_id, item.file_path, item.name)?
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
            item:                    Some(format!("{kind_label} {name}")),
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
    sink: &mut FindingsSink,
) -> Result<bool> {
    match annotation.syntax() {
        VisibilitySyntax::Crate | VisibilitySyntax::InCrate => record_forbidden_pub_crate(
            ctx,
            item,
            annotation,
            finding_context,
            parent_facade_analysis,
            sink,
        ),
        VisibilitySyntax::InParent | VisibilitySyntax::InCurrent | VisibilitySyntax::InPath(_) => {
            record_forbidden_pub_in_crate(
                ctx,
                item,
                annotation,
                finding_context,
                parent_facade_analysis,
                sink,
            )
        },
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current => Ok(false),
    }
}

fn record_forbidden_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let policy_permits_pub_crate = policy::allow_pub_crate_by_policy(
        finding_context.crate_kind,
        finding_context.module_location,
        finding_context.parent_visibility,
    );
    let resolved_chain_reach = parent_facade_analysis.and_then(|analysis| {
        let FacadeChainResolution::Resolved { required } = analysis.chain else {
            return None;
        };
        Some(required)
    });
    let pub_crate_is_permitted = resolved_chain_reach
        .map_or(policy_permits_pub_crate, |required| {
            reach_is_pub_crate(required, ctx)
        });
    if matches!(annotation.syntax(), VisibilitySyntax::Crate) && pub_crate_is_permitted {
        return Ok(false);
    }

    let exact_restricted_boundary = resolved_chain_reach.filter(|required| {
        !reach_is_pub_crate(*required, ctx) && required.to_source(ctx.tcx) != "pub"
    });
    let (message, suggestion) = if let Some(blocker_text) =
        parent_facade_blocker_text(ctx, parent_facade_analysis)
    {
        (
            format!(
                "use of `{}` is forbidden by policy",
                annotation.display_source()
            ),
            Some(blocker_text),
        )
    } else if let Some(required) = exact_restricted_boundary {
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
        (
            message,
            Some(format!("consider using: `{}`", required.to_source(ctx.tcx))),
        )
    } else if matches!(annotation.syntax(), VisibilitySyntax::InCrate) && pub_crate_is_permitted {
        (
            String::from("`pub(in crate)` is a redundant spelling of `pub(crate)`"),
            Some(String::from("consider using: `pub(crate)`")),
        )
    } else {
        let signature_exposure =
            policy::has_signature_exposure_allowance(ctx, item.def_id, item.file_path, item.name)?
                .into();
        let boundary_path = parent_facade_analysis
            .and_then(|analysis| resolved_facade_boundary_path(analysis, ctx));
        (
            format!(
                "use of `{}` is forbidden by policy",
                annotation.display_source()
            ),
            Some(policy::forbidden_pub_crate_suggestion(
                finding_context.module_location,
                signature_exposure,
                parent_facade_analysis.map(|analysis| parent_facade_reach(analysis, ctx)),
                boundary_path.as_deref(),
            )),
        )
    };
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::ForbiddenPubCrate,
            item: None,
            message,
            suggestion,
            fix_support: FixSupport::None,
            related: None,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(true)
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    finding_context: &VisibilityFindingContext,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    // `check_field` supplies no `ParentFacadeAnalysis`, so fields cannot satisfy
    // this declaration-only acceptance path even though their category is `Declaration`.
    if exact_pub_in_boundary_is_allowed(ctx, item, annotation, parent_facade_analysis) {
        return Ok(false);
    }

    if !matches!(
        annotation.syntax(),
        VisibilitySyntax::InParent | VisibilitySyntax::InCurrent | VisibilitySyntax::InPath(_)
    ) {
        return Ok(false);
    }
    let annotation_reach = annotation.reach(item.def_id, ctx.tcx);
    let advice = match parent_facade_analysis.map(|analysis| analysis.chain) {
        Some(FacadeChainResolution::Unresolvable {
            blocker: FacadeChainBlocker::Glob(_),
        }) => glob_blocked_pub_in_advice(ctx, parent_facade_analysis),
        Some(FacadeChainResolution::Unresolvable { .. }) => {
            unresolvable_pub_in_advice(ctx, annotation, annotation_reach, parent_facade_analysis)
        },
        Some(FacadeChainResolution::Resolved { required }) => {
            resolved_pub_in_advice(ctx, item, annotation, annotation_reach, required)
        },
        None => no_facade_pub_in_advice(
            ctx,
            item,
            annotation,
            annotation_reach,
            finding_context,
            sink,
        ),
    };
    let (item_def_path, narrower_scope_def_path) = advice
        .caller_metadata
        .map_or((None, None), |(item_def_path, item_module)| {
            (Some(item_def_path), Some(item_module))
        });
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
            item: None,
            message: advice.message,
            suggestion: advice.suggestion,
            fix_support: FixSupport::None,
            related: None,
            visibility_annotation: Some(annotation.source().to_string()),
            item_def_path,
            narrower_scope_def_path,
        },
    )?);
    Ok(true)
}

struct ForbiddenPubInAdvice {
    message:         String,
    suggestion:      Option<String>,
    caller_metadata: Option<(String, String)>,
}

fn exact_pub_in_boundary_is_allowed(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> bool {
    matches!(
        ctx.settings.visibility_config.pub_in_path,
        PubInPath::Permitted | PubInPath::Required
    ) && item.category == ItemCategory::Declaration
        && matches!(
            annotation.syntax(),
            VisibilitySyntax::InPath(PathSpelling::CrateRooted)
        )
        && parent_facade_analysis.is_some_and(|analysis| {
            matches!(
                analysis.chain,
                FacadeChainResolution::Resolved { required }
                    if annotation.reach(item.def_id, ctx.tcx).compare(required, ctx.tcx)
                        == Some(Ordering::Equal)
            )
        })
}

fn default_pub_in_repair(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
) -> Option<String> {
    match annotation.syntax() {
        VisibilitySyntax::InParent => Some(String::from("consider using: `pub(super)`")),
        VisibilitySyntax::InCurrent => Some(String::from("consider using: `pub(self)`")),
        VisibilitySyntax::InPath(PathSpelling::Relative) => Some(format!(
            "consider using: `{}`",
            annotation_reach.to_source(ctx.tcx)
        )),
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Crate
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InPath(PathSpelling::CrateRooted) => None,
    }
}

fn glob_blocked_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> ForbiddenPubInAdvice {
    ForbiddenPubInAdvice {
        message:         String::from(
            "parent facade does not provide a resolvable visibility boundary",
        ),
        suggestion:      parent_facade_blocker_text(ctx, parent_facade_analysis)
            .map(|text| format!("{text} before using `pub(in ...)`")),
        caller_metadata: None,
    }
}

fn unresolvable_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
) -> ForbiddenPubInAdvice {
    ForbiddenPubInAdvice {
        message:         format!(
            "use of `{}` is forbidden by policy",
            annotation.display_source()
        ),
        suggestion:      combine_blocker_and_repair(
            parent_facade_blocker_text(ctx, parent_facade_analysis),
            default_pub_in_repair(ctx, annotation, annotation_reach),
        ),
        caller_metadata: None,
    }
}

fn resolved_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    required: VisibilityReach,
) -> ForbiddenPubInAdvice {
    let comparison = annotation_reach.compare(required, ctx.tcx);
    let (message, suggestion) =
        if comparison == Some(Ordering::Equal) && reach_is_pub_crate(required, ctx) {
            (
                String::from("parent facade caps reach at `pub(crate)`"),
                Some(String::from("consider using: `pub(crate)`")),
            )
        } else if matches!(
            annotation.syntax(),
            VisibilitySyntax::InPath(PathSpelling::CrateRooted)
        ) && comparison == Some(Ordering::Equal)
            && item.category == ItemCategory::Declaration
            && matches!(
                ctx.settings.visibility_config.pub_in_path,
                PubInPath::Forbidden
            )
        {
            (
                format!(
                    "use of `{}` is disabled by project visibility policy",
                    annotation.display_source()
                ),
                Some(String::from(
                    "consider using: `pub`; or set `pub_in_path = \"permitted\"`",
                )),
            )
        } else if comparison == Some(Ordering::Equal) {
            exact_restricted_spelling_advice(ctx, annotation, required)
        } else {
            let message = if comparison == Some(Ordering::Greater) {
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
            (
                message,
                Some(format!("consider using: `{}`", required.to_source(ctx.tcx))),
            )
        };
    ForbiddenPubInAdvice {
        message,
        suggestion,
        caller_metadata: None,
    }
}

fn exact_restricted_spelling_advice(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    required: VisibilityReach,
) -> (String, Option<String>) {
    match annotation.syntax() {
        VisibilitySyntax::InParent => (
            String::from("`pub(in super)` is a redundant spelling of `pub(super)`"),
            Some(String::from("consider using: `pub(super)`")),
        ),
        VisibilitySyntax::InCurrent => (
            String::from("`pub(in self)` is a redundant spelling of `pub(self)`"),
            Some(String::from("consider using: `pub(self)`")),
        ),
        VisibilitySyntax::InPath(PathSpelling::Relative) => (
            format!(
                "use of `{}` does not use the canonical crate-rooted boundary",
                annotation.display_source()
            ),
            Some(format!("consider using: `{}`", required.to_source(ctx.tcx))),
        ),
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Crate
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InPath(PathSpelling::CrateRooted) => (
            format!(
                "use of `{}` is forbidden by policy",
                annotation.display_source()
            ),
            None,
        ),
    }
}

fn no_facade_pub_in_advice(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    annotation_reach: VisibilityReach,
    finding_context: &VisibilityFindingContext,
    sink: &FindingsSink,
) -> ForbiddenPubInAdvice {
    let default_repair = default_pub_in_repair(ctx, annotation, annotation_reach);
    if matches!(annotation.syntax(), VisibilitySyntax::InParent) {
        return ForbiddenPubInAdvice {
            message:         String::from(
                "`pub(in super)` is a redundant spelling of `pub(super)`",
            ),
            suggestion:      default_repair,
            caller_metadata: None,
        };
    }
    if matches!(annotation.syntax(), VisibilitySyntax::InCurrent) {
        return ForbiddenPubInAdvice {
            message:         String::from("`pub(in self)` is a redundant spelling of `pub(self)`"),
            suggestion:      default_repair,
            caller_metadata: None,
        };
    }

    if item.category == ItemCategory::Use {
        // Resolved paths name the imported target, not the local `use` item,
        // so an empty caller set cannot prove that its alias has no users.
        return ForbiddenPubInAdvice {
            message:         format!(
                "use of `{}` on a `use` item is forbidden by policy",
                annotation.display_source()
            ),
            suggestion:      None,
            caller_metadata: None,
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
    let callers = sink
        .use_sites
        .iter()
        .filter(|site| site.target_def_path == item_def_path)
        .map(|site| site.caller_module_def_path.clone())
        .collect::<BTreeSet<_>>();
    let advice = policy::classify_no_facade_callers(&item_module, parent_scope, &callers);
    let message = if matches!(advice, policy::NoFacadeAdvice::StructuralMigration) {
        String::from(
            "no visibility annotation allowed by policy preserves this item's current callers",
        )
    } else {
        format!(
            "use of `{}` outside an exact facade boundary is forbidden by policy",
            annotation.display_source()
        )
    };
    ForbiddenPubInAdvice {
        message,
        suggestion: Some(policy::no_facade_suggestion(advice, &boundary_path)),
        caller_metadata: Some((item_def_path, item_module)),
    }
}

fn combine_blocker_and_repair(
    blocker_text: Option<String>,
    repair_text: Option<String>,
) -> Option<String> {
    match (blocker_text, repair_text) {
        (Some(blocker), Some(repair)) => Some(format!("{blocker} — {repair}")),
        (Some(blocker), None) => Some(blocker),
        (None, repair) => repair,
    }
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
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            diagnostic_code:         DiagnosticCode::NarrowToPubCrate,
            item:                    Some(format!("{kind_label} {name}")),
            message:                 String::from(
                "item is not re-exported by the crate root — use `pub(crate)`",
            ),
            suggestion:              Some(String::from("consider using: `pub(crate)`")),
            fix_support:             FixSupport::NarrowToPubCrate,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn maybe_record_narrow_to_pub_crate_nested(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
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
    if parent_facade_analysis.nearest.spelling_conflict || !reach_is_pub_crate(required, ctx) {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            diagnostic_code:         DiagnosticCode::NarrowToPubCrate,
            item:                    Some(format!("{kind_label} {name}")),
            message:                 String::from(
                "parent facade caps reach at `pub(crate)` — narrow source to match",
            ),
            suggestion:              Some(String::from("consider using: `pub(crate)`")),
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
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match policy::classify_suspicious_pub(ctx, input, parent_facade_analysis)? {
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
                kind_label,
                annotation,
                parent_facade_analysis,
                SuspiciousWarning {
                    fix_support,
                    related,
                    stale_parent_pub_use,
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
            item: input.name.map(|name| {
                facade_use.map_or_else(
                    || format!("re-export {name}"),
                    |syntax| format!("{syntax} {name}"),
                )
            }),
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
    kind_label: &str,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    warning: SuspiciousWarning,
    sink: &mut FindingsSink,
) -> Result<()> {
    let SuspiciousWarning {
        fix_support,
        related,
        stale_parent_pub_use,
    } = warning;
    let restricted_annotation = !matches!(annotation.syntax(), VisibilitySyntax::Public);
    let fix_support = if restricted_annotation {
        FixSupport::None
    } else {
        facade_chain_fix_support(parent_facade_analysis, fix_support)
    };
    let suggestion = suspicious_pub_suggestion(
        ctx,
        annotation,
        parent_facade_analysis,
        stale_parent_pub_use.as_ref(),
    );
    let item_def_path = Some(use_sites::def_path_string(ctx.tcx, input.def_id));
    let facade_scope = parent_facade_analysis
        .and_then(|analysis| resolved_facade_boundary_path(analysis, ctx))
        .and_then(|path| path.strip_prefix("crate::").map(String::from));
    let narrower_scope_def_path = Some(
        facade_scope.unwrap_or_else(|| use_sites::parent_module_def_path(ctx.tcx, input.def_id)),
    );
    sink.findings.push(source::build_finding(
        ctx.tcx,
        input.file_path,
        input.highlight_span,
        FindingParams {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::SuspiciousPub,
            item: input.name.map(|name| format!("{kind_label} {name}")),
            message: policy::suspicious_pub_note(input.crate_kind, kind_label),
            suggestion: Some(suggestion),
            fix_support,
            related,
            visibility_annotation: None,
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

fn suspicious_pub_suggestion(
    ctx: &VisibilityContext<'_, '_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    stale_parent_facade: Option<&ParentFacadeExportStatus>,
) -> String {
    if stale_parent_facade.is_some() && !matches!(annotation.syntax(), VisibilitySyntax::Public) {
        return format!(
            "remove the parent facade and the now-unneeded `{}` annotation",
            annotation.display_source()
        );
    }
    if let Some(parent_facade_analysis) = parent_facade_analysis
        && let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain
    {
        return format!("consider using: `{}`", required.to_source(ctx.tcx));
    }
    String::from("consider using: `pub(super)`")
}

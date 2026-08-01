use std::cmp::Ordering;
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

    if matches!(annotation.syntax(), VisibilitySyntax::Public)
        && finding_context.logical_module_depth > 1
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
            },
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
            record_forbidden_pub_in_crate(ctx, item, annotation, parent_facade_analysis, sink)
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
    let pub_crate_is_permitted =
        policy::allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        ) || parent_facade_chain_reaches_crate(parent_facade_analysis, ctx);
    if matches!(annotation.syntax(), VisibilitySyntax::Crate) && pub_crate_is_permitted {
        return Ok(false);
    }
    let suggestion = if let Some(blocker_text) =
        parent_facade_blocker_text(ctx, parent_facade_analysis)
    {
        Some(blocker_text)
    } else {
        let signature_exposure =
            policy::has_signature_exposure_allowance(ctx, item.def_id, item.file_path, item.name)?
                .into();
        let repair_text =
            if matches!(annotation.syntax(), VisibilitySyntax::InCrate) && pub_crate_is_permitted {
                String::from("consider using: `pub(crate)`")
            } else {
                wider_chain_boundary_repair(parent_facade_analysis, ctx).unwrap_or_else(|| {
                    policy::forbidden_pub_crate_suggestion(
                        finding_context.module_location,
                        signature_exposure,
                        parent_facade_analysis.map(|analysis| parent_facade_reach(analysis, ctx)),
                    )
                    .to_string()
                })
            };
        Some(repair_text)
    };
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::ForbiddenPubCrate,
            item: None,
            message: format!("use of `{}` is forbidden by policy", annotation.source()),
            suggestion,
            fix_support: FixSupport::None,
            related: None,
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
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let repair_text = match annotation.syntax() {
        VisibilitySyntax::InParent => Some(String::from("consider using: `pub(super)`")),
        VisibilitySyntax::InCurrent => Some(String::from("consider using: `pub(self)`")),
        VisibilitySyntax::InPath(PathSpelling::CrateRooted) => None,
        VisibilitySyntax::InPath(PathSpelling::Relative) => Some(format!(
            "consider using: `{}`",
            annotation.reach(item.def_id, ctx.tcx).to_source(ctx.tcx)
        )),
        VisibilitySyntax::Private
        | VisibilitySyntax::Public
        | VisibilitySyntax::Crate
        | VisibilitySyntax::Parent
        | VisibilitySyntax::Current
        | VisibilitySyntax::InCrate => return Ok(false),
    };
    let suggestion = combine_blocker_and_repair(
        parent_facade_blocker_text(ctx, parent_facade_analysis),
        repair_text,
    );
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
            item: None,
            message: format!("use of `{}` is forbidden by policy", annotation.source()),
            suggestion,
            fix_support: FixSupport::None,
            related: None,
            item_def_path: None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(true)
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
    // A crate-root `pub mod prelude;` is exempt by default (global `allow_prelude_pub_mod`).
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

fn parent_facade_chain_reaches_crate(
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    ctx: &VisibilityContext<'_, '_>,
) -> bool {
    let Some(parent_facade_analysis) = parent_facade_analysis else {
        return false;
    };
    let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain else {
        return false;
    };
    reach_is_pub_crate(required, ctx)
}

fn wider_chain_boundary_repair(
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    ctx: &VisibilityContext<'_, '_>,
) -> Option<String> {
    let parent_facade_analysis = parent_facade_analysis?;
    let FacadeChainResolution::Resolved { required } = parent_facade_analysis.chain else {
        return None;
    };
    let nearest = VisibilityReach::from(parent_facade_analysis.nearest.selected.visibility);
    (required.compare(nearest, ctx.tcx) == Some(Ordering::Greater))
        .then(|| format!("consider using: `{}`", required.to_source(ctx.tcx)))
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
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match policy::classify_suspicious_pub(ctx, input, parent_facade_analysis)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
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
                        |syntax| {
                            format!("parent module `{syntax}` is acting as an internal facade")
                        },
                    ),
                    suggestion: None,
                    fix_support: FixSupport::InternalParentFacade,
                    related,
                    item_def_path: None,
                    narrower_scope_def_path: None,
                },
            )?);
        },
        SuspiciousPubAssessment::Warn {
            fix_support,
            related,
            stale_parent_pub_use,
        } => {
            let fix_support = facade_chain_fix_support(parent_facade_analysis, fix_support);
            let item_def_path = Some(use_sites::def_path_string(ctx.tcx, input.def_id));
            let narrower_scope_def_path =
                Some(use_sites::parent_module_def_path(ctx.tcx, input.def_id));
            sink.findings.push(source::build_finding(
                ctx.tcx,
                input.file_path,
                input.highlight_span,
                FindingParams {
                    severity: Severity::Warning,
                    diagnostic_code: DiagnosticCode::SuspiciousPub,
                    item: input.name.map(|name| format!("{kind_label} {name}")),
                    message: policy::suspicious_pub_note(input.crate_kind, kind_label),
                    suggestion: None,
                    fix_support,
                    related,
                    item_def_path,
                    narrower_scope_def_path,
                },
            )?);
            if let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.name)
                && fix_support == FixSupport::PubUse
            {
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
                    return Ok(());
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
        },
    }
    Ok(())
}

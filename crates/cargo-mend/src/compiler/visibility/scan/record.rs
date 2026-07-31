use std::ffi::OsStr;

use anyhow::Result;
use rustc_middle::middle::privacy::Level;

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
use super::classify::SignatureExposure;
use super::classify::VisibilityFindingContext;
use crate::compiler::constants::PRELUDE_MODULE_NAME;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentFacadeVisibility;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredPubUseFixFact;
use crate::compiler::visibility::annotation::PathSpelling;
use crate::compiler::visibility::annotation::VisibilityAnnotation;
use crate::compiler::visibility::annotation::VisibilitySyntax;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::compiler::visibility::use_sites::FacadeVisibility;
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
    let parent_facade_reach = match annotation.syntax() {
        VisibilitySyntax::Crate | VisibilitySyntax::InCrate => {
            resolve_parent_facade_reach(ctx, item)
        },
        _ => None,
    };

    if record_forbidden_visibility_annotation(
        ctx,
        item,
        &annotation,
        &finding_context,
        parent_facade_reach,
        sink,
    )? {
        return Ok(());
    }
    record_review_pub_mod(ctx, item, &annotation, &finding_context, sink)?;
    maybe_record_unused_pub(ctx, item, &annotation, &finding_context, sink)?;

    if matches!(annotation.syntax(), VisibilitySyntax::Public)
        && finding_context.parent_visibility == ParentVisibility::Private
        && finding_context.logical_module_depth == 1
        && policy::allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        )
    {
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if matches!(annotation.syntax(), VisibilitySyntax::Public)
        && finding_context.parent_visibility == ParentVisibility::Private
        && finding_context.logical_module_depth > 1
        && finding_context.crate_kind != CrateKind::IntegrationTest
    {
        maybe_record_narrow_to_pub_crate_nested(ctx, item, sink)?;
    }

    if matches!(annotation.syntax(), VisibilitySyntax::Public)
        && finding_context.logical_module_depth > 1
    {
        maybe_record_suspicious_pub(
            ctx,
            &SuspiciousPubInput {
                def_id:            item.def_id,
                facade_subject:    item.facade_subject,
                file_path:         item.file_path,
                config_rel_path:   finding_context.config_rel_path.as_deref(),
                parent_visibility: finding_context.parent_visibility,
                module_location:   finding_context.module_location,
                crate_kind:        finding_context.crate_kind,
                kind_label:        item.kind_label,
                name:              item.name,
                highlight_span:    item.highlight_span,
            },
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
    if parent_facade_exports_item(ctx, item)
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
    parent_facade_reach: Option<ParentFacadeReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    match annotation.syntax() {
        VisibilitySyntax::Crate | VisibilitySyntax::InCrate => record_forbidden_pub_crate(
            ctx,
            item,
            annotation,
            finding_context,
            parent_facade_reach,
            sink,
        ),
        VisibilitySyntax::InParent | VisibilitySyntax::InCurrent | VisibilitySyntax::InPath(_) => {
            record_forbidden_pub_in_crate(ctx, item, annotation, sink)
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
    parent_facade_reach: Option<ParentFacadeReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let pub_crate_is_permitted = policy::allow_pub_crate_by_policy(
        finding_context.crate_kind,
        finding_context.module_location,
        finding_context.parent_visibility,
    ) || matches!(
        parent_facade_reach,
        Some(ParentFacadeReach {
            visibility: ParentFacadeVisibility::Crate,
            ..
        })
    );
    if matches!(annotation.syntax(), VisibilitySyntax::Crate) && pub_crate_is_permitted {
        return Ok(false);
    }
    let signature_exposure: SignatureExposure =
        policy::has_signature_exposure_allowance(ctx, item.def_id, item.file_path, item.name)?
            .into();
    let suggestion =
        if matches!(annotation.syntax(), VisibilitySyntax::InCrate) && pub_crate_is_permitted {
            String::from("consider using: `pub(crate)`")
        } else {
            policy::forbidden_pub_crate_suggestion(
                finding_context.module_location,
                signature_exposure,
                parent_facade_reach,
            )
            .to_string()
        };
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ForbiddenPubCrate,
            item:                    None,
            message:                 format!(
                "use of `{}` is forbidden by policy",
                annotation.source()
            ),
            suggestion:              Some(suggestion),
            fix_support:             FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(true)
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let suggestion = match annotation.syntax() {
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
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    let Some(occurrences) =
        ctx.reexport_index
            .parent_facade_occurrences(ctx.tcx, item.def_id, item.facade_subject)
    else {
        return Ok(());
    };
    let occurrence = occurrences.selected;
    if occurrences.spelling_conflict
        || occurrence.facade_visibility != FacadeVisibility::Crate
        || occurrence.facade_spelling == ParentFacadeSpelling::Super
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

/// Resolved reach and source-spelling metadata for the parent module's `use`
/// re-export of this item, when the parent re-exports it at all.
fn resolve_parent_facade_reach(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> Option<ParentFacadeReach> {
    ctx.reexport_index
        .parent_facade_occurrences(ctx.tcx, item.def_id, item.facade_subject)
        .map(|occurrences| ParentFacadeReach {
            visibility:        parent_facade_visibility(occurrences.selected.facade_visibility),
            spelling:          occurrences.selected.facade_spelling,
            spelling_conflict: occurrences.spelling_conflict,
        })
}

fn parent_facade_exports_item(ctx: &VisibilityContext<'_, '_>, item: &ItemInfo<'_>) -> bool {
    ctx.reexport_index
        .has_parent_facade(ctx.tcx, item.def_id, item.facade_subject)
}

const fn parent_facade_visibility(visibility: FacadeVisibility) -> ParentFacadeVisibility {
    match visibility {
        FacadeVisibility::Public => ParentFacadeVisibility::Public,
        FacadeVisibility::Crate => ParentFacadeVisibility::Crate,
        FacadeVisibility::Super => ParentFacadeVisibility::Super,
        FacadeVisibility::Unrecognized => ParentFacadeVisibility::Unrecognized,
    }
}

fn maybe_record_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match policy::classify_suspicious_pub(ctx, input)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
            let Some(status) = policy::parent_facade_export_status(
                ctx,
                input.def_id,
                input.facade_subject,
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

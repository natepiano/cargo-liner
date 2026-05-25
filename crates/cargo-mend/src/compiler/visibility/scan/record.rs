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
use super::classify::VisibilityFindingContext;
use crate::compiler::RUST_MODULE_FILE_STEM;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeVisibility;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredPubUseFixFact;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::config::DiagnosticCode;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::rust_syntax::PUB_CRATE_VISIBILITY;
use crate::rust_syntax::PUB_IN_CRATE_VISIBILITY_PREFIX;
use crate::rust_syntax::PUB_VISIBILITY_TOKEN;

pub(super) fn record_visibility_findings(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let finding_context = classify::visibility_finding_context(ctx, item);

    record_forbidden_pub_crate(ctx, item, &finding_context, sink)?;
    record_forbidden_pub_in_crate(ctx, item, sink)?;
    record_review_pub_mod(ctx, item, &finding_context, sink)?;
    maybe_record_unused_pub(ctx, item, &finding_context, sink)?;

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && finding_context.parent_visibility == ParentVisibility::Private
        && policy::is_top_level_module_file(ctx.source_root, ctx.root_module, item.file_path)
        && policy::allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        )
    {
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && finding_context.parent_visibility == ParentVisibility::Private
        && !policy::is_top_level_module_file(ctx.source_root, ctx.root_module, item.file_path)
        && finding_context.crate_kind != CrateKind::IntegrationTest
    {
        maybe_record_narrow_to_pub_crate_nested(ctx, item, sink)?;
    }

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && !policy::is_boundary_file(ctx.source_root, ctx.root_module, item.file_path)
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
            sink,
        )?;
    }
    Ok(())
}

fn maybe_record_unused_pub(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.vis_text != PUB_VISIBILITY_TOKEN || item.category == ItemCategory::Module {
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
    if parent_facade_exports_item(ctx, item)?
        || facade::parent_facade_has_glob_export(ctx.source_cache, ctx.source_root, item.file_path)?
        || facade::path_exists_outside_child_module(
            ctx.source_cache,
            ctx.source_root,
            &use_sites::parent_module_path_segments(ctx.tcx, item.def_id),
            name,
        )
        || policy::has_signature_exposure_allowance(ctx, item.file_path, item.name)?
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
            fixability:              FixSupport::UnusedPub,
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

fn record_forbidden_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if !matches!(item.vis_text, PUB_CRATE_VISIBILITY) {
        return Ok(());
    }
    if policy::allow_pub_crate_by_policy(
        finding_context.crate_kind,
        finding_context.module_location,
        finding_context.parent_visibility,
    ) {
        return Ok(());
    }
    if parent_facade_caps_at_pub_crate(ctx, item)? {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ForbiddenPubCrate,
            item:                    None,
            message:                 "use of `pub(crate)` is forbidden by policy".to_string(),
            suggestion:              Some(
                policy::forbidden_pub_crate_help(finding_context.module_location).to_string(),
            ),
            fixability:              FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if !item.vis_text.starts_with(PUB_IN_CRATE_VISIBILITY_PREFIX) {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ForbiddenPubInCrate,
            item:                    None,
            message:                 "use of `pub(in crate::...)` is forbidden by policy"
                .to_string(),
            suggestion:              None,
            fixability:              FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn record_review_pub_mod(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.category != ItemCategory::Module || !item.vis_text.starts_with(PUB_VISIBILITY_TOKEN) {
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
            fixability:              FixSupport::None,
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
    if ctx
        .effective_visibilities
        .is_public_at_level(item.def_id, Level::Reachable)
    {
        return Ok(());
    }
    if facade::root_module_exports_item(ctx.source_cache, ctx.root_module, item.file_path, name) {
        return Ok(());
    }
    if let Some(self_name) = &item.impl_self_name
        && facade::root_module_exports_item(
            ctx.source_cache,
            ctx.root_module,
            item.file_path,
            self_name,
        )
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
            fixability:              FixSupport::NarrowToPubCrate,
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
    if !parent_facade_caps_at_pub_crate(ctx, item)? {
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
            fixability:              FixSupport::NarrowToPubCrate,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn parent_facade_caps_at_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> Result<bool> {
    let Some(name) = item.name else {
        return Ok(false);
    };
    let status = facade::parent_facade_export_status(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        item.file_path,
        name,
    )?;
    Ok(matches!(
        status.as_ref().map(|s| s.visibility),
        Some(ParentFacadeVisibility::Crate)
    ))
}

fn parent_facade_exports_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> Result<bool> {
    let Some(name) = item.name else {
        return Ok(false);
    };
    Ok(facade::parent_facade_export_status(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        item.file_path,
        name,
    )?
    .is_some())
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
            let Some(status) = input
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
                .flatten()
            else {
                return Ok(());
            };
            sink.findings.push(source::build_line_finding(
                ctx.source_cache,
                &status.parent_path,
                status.parent_line,
                FindingParams {
                    severity: Severity::Warning,
                    diagnostic_code: DiagnosticCode::InternalParentPubUseFacade,
                    item: input.name.map(|name| format!("pub use {name}")),
                    message: String::from(
                        "this `pub use` is used inside its parent module subtree",
                    ),
                    suggestion: None,
                    fixability: FixSupport::InternalParentFacade,
                    related,
                    item_def_path: None,
                    narrower_scope_def_path: None,
                },
            )?);
        },
        SuspiciousPubAssessment::Warn {
            fixability,
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
                    fixability,
                    related,
                    item_def_path,
                    narrower_scope_def_path,
                },
            )?);
            if let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.name)
                && fixability == FixSupport::PubUse
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
                    .filter(|stem| *stem != RUST_MODULE_FILE_STEM)
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

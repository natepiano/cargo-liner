use anyhow::Result;
use rustc_hir::ForeignItem;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;

use super::FindingParams;
use super::ItemCategory;
use super::ItemInfo;
use super::VisibilityContext;
use super::record;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::source;
use crate::config::DiagnosticCode;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::rust_syntax::PUB_VISIBILITY_TOKEN;

pub(super) fn analyze_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &Item<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let name = item.kind.ident().as_ref().map(ToString::to_string);

    if vis_text == PUB_VISIBILITY_TOKEN
        && policy::is_boundary_file(ctx.source_root, ctx.root_module, &file_path)
        && matches!(item.kind, ItemKind::Use(..))
        && source::use_item_contains_glob(ctx.tcx, item.span)?
    {
        sink.findings.push(source::build_finding(
            ctx.tcx,
            &file_path,
            item.span,
            FindingParams {
                severity:                Severity::Warning,
                code:                    DiagnosticCode::WildcardParentPubUse,
                item:                    None,
                message:                 String::new(),
                suggestion:              None,
                fixability:              FixSupport::None,
                related:                 None,
                item_def_path:           None,
                narrower_scope_def_path: None,
            },
        )?);
    }

    record::record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     source::item_kind_label(item.kind),
            name:           name.as_deref(),
            highlight_span: source::highlight_span(
                item.vis_span,
                item.kind.ident().map(|ident| ident.span),
            ),
            category:       if matches!(item.kind, ItemKind::Mod(..)) {
                ItemCategory::Module
            } else {
                ItemCategory::NonModule
            },
            impl_self_name: None,
        },
        sink,
    )
}

pub(super) fn analyze_impl_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ImplItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(vis_span) = item.vis_span() else {
        return Ok(());
    };
    if item.span.from_expansion() || vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(ctx.tcx, vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, vis_span)? else {
        return Ok(());
    };

    let name = item.ident.to_string();
    let impl_self_name = source::impl_self_type_name_from_tcx(ctx.tcx, item.owner_id.def_id);

    record::record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id: item.owner_id.def_id,
            file_path: &file_path,
            vis_text: &vis_text,
            kind_label: Some(source::impl_item_kind_label(item.kind)),
            name: Some(name.as_str()),
            highlight_span: source::highlight_span(vis_span, Some(item.ident.span)),
            category: ItemCategory::NonModule,
            impl_self_name,
        },
        sink,
    )
}

pub(super) fn analyze_foreign_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ForeignItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let name = item.ident.to_string();

    record::record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     Some(source::foreign_item_kind_label(item.kind)),
            name:           Some(name.as_str()),
            highlight_span: source::highlight_span(item.vis_span, Some(item.ident.span)),
            category:       ItemCategory::NonModule,
            impl_self_name: None,
        },
        sink,
    )
}

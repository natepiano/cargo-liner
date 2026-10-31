use anyhow::Result;
use rustc_hir::ForeignItem;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;

use super::FindingParams;
use super::ItemCategory;
use super::ItemInfo;
use super::VisibilityContext;
use super::pub_use_outside_subtree;
use super::record;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::visibility::policy;
use crate::compiler::visibility::source;
use crate::config::DiagnosticCode;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

pub(super) fn visit_item(
    visibility_context: &VisibilityContext<'_, '_>,
    item: &Item<'_>,
    findings_sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(visibility_context.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(visibility_text) = source::visibility_text(visibility_context.tcx, item.vis_span)?
    else {
        return Ok(());
    };

    let name = item.kind.ident().as_ref().map(ToString::to_string);
    let owner_module = visibility_context
        .tcx
        .parent_module_from_def_id(item.owner_id.def_id);

    if visibility_text == "pub"
        && policy::module_depth(visibility_context.tcx, owner_module.to_local_def_id()) <= 1
        && matches!(item.kind, ItemKind::Use(..))
        && source::use_item_contains_glob(visibility_context.tcx, item.span)?
    {
        findings_sink.findings.push(source::build_finding(
            visibility_context.tcx,
            &file_path,
            item.span,
            FindingParams {
                severity:                Severity::Warning,
                diagnostic_code:         DiagnosticCode::WildcardParentPubUse,
                item:                    None,
                message:                 String::new(),
                suggestion:              None,
                fix_support:             FixSupport::None,
                related:                 None,
                visibility_annotation:   None,
                item_def_path:           None,
                narrower_scope_def_path: None,
                exact_boundary_spelling: ExactBoundarySpelling::CratePath,
            },
        )?);
    }

    pub_use_outside_subtree::record(
        visibility_context.tcx,
        item,
        &file_path,
        visibility_context.root_module,
        findings_sink,
    )?;

    record::record_visibility_findings(
        visibility_context,
        &ItemInfo {
            def_id:          item.owner_id.def_id,
            file_path:       &file_path,
            visibility_text: &visibility_text,
            kind_label:      source::item_kind_label(item.kind),
            name:            name.as_deref(),
            highlight_span:  source::highlight_span(
                item.vis_span,
                item.kind.ident().map(|ident| ident.span),
            ),
            category:        match item.kind {
                ItemKind::Mod(..) => ItemCategory::Module,
                ItemKind::Use(..) => ItemCategory::Use,
                _ => ItemCategory::Declaration,
            },
            facade_subject:  item.owner_id.def_id,
        },
        findings_sink,
    )
}

pub(super) fn visit_impl_item(
    visibility_context: &VisibilityContext<'_, '_>,
    item: &ImplItem<'_>,
    findings_sink: &mut FindingsSink,
) -> Result<()> {
    let Some(visibility_span) = item.vis_span() else {
        return Ok(());
    };
    if item.span.from_expansion() || visibility_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(visibility_context.tcx, visibility_span) else {
        return Ok(());
    };
    let Some(visibility_text) = source::visibility_text(visibility_context.tcx, visibility_span)?
    else {
        return Ok(());
    };

    let name = item.ident.to_string();
    let facade_subject = visibility_context
        .reexport_index
        .facade_subject(item.owner_id.def_id);

    record::record_visibility_findings(
        visibility_context,
        &ItemInfo {
            def_id: item.owner_id.def_id,
            file_path: &file_path,
            visibility_text: &visibility_text,
            kind_label: Some(source::impl_item_kind_label(item.kind)),
            name: Some(name.as_str()),
            highlight_span: source::highlight_span(visibility_span, Some(item.ident.span)),
            category: ItemCategory::Declaration,
            facade_subject,
        },
        findings_sink,
    )
}

pub(super) fn visit_foreign_item(
    visibility_context: &VisibilityContext<'_, '_>,
    item: &ForeignItem<'_>,
    findings_sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(visibility_context.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(visibility_text) = source::visibility_text(visibility_context.tcx, item.vis_span)?
    else {
        return Ok(());
    };

    let name = item.ident.to_string();

    record::record_visibility_findings(
        visibility_context,
        &ItemInfo {
            def_id:          item.owner_id.def_id,
            file_path:       &file_path,
            visibility_text: &visibility_text,
            kind_label:      Some(source::foreign_item_kind_label(item.kind)),
            name:            Some(name.as_str()),
            highlight_span:  source::highlight_span(item.vis_span, Some(item.ident.span)),
            category:        ItemCategory::Declaration,
            facade_subject:  item.owner_id.def_id,
        },
        findings_sink,
    )
}

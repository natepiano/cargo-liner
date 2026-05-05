//! Detects struct/union/enum-variant fields whose declared visibility is
//! strictly wider than their effective reach, accounting for `pub use`
//! re-exports of the containing type.
//!
//! Comparison: `tcx.visibility(field_def_id)` (the literal annotation on
//! the field) vs the field's effective visibility, which rustc caps at
//! the containing type's reach including re-exports. If the declared
//! annotation claims more reach than the effective reach permits, the
//! annotation is dead — emit a finding suggesting the type's literal
//! visibility token as the narrower replacement.

use anyhow::Result;
use rustc_hir::FieldDef;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::VariantData;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::Span;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;

type DefIdVisibility = Visibility<DefId>;

use super::scan::FindingParams;
use super::scan::VisibilityContext;
use super::source;
use crate::compiler::persistence::FindingsSink;
use crate::config::DiagnosticCode;
use crate::diagnostics::Severity;
use crate::fix_support::FixSupport;

pub(super) fn check_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &Item<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let type_def_id = item.owner_id.def_id;
    match &item.kind {
        ItemKind::Struct(_, _, variant_data) | ItemKind::Union(_, _, variant_data) => {
            check_variant_data(ctx, type_def_id, variant_data, sink)?;
        },
        ItemKind::Enum(_, _, enum_def) => {
            for variant in enum_def.variants {
                check_variant_data(ctx, type_def_id, &variant.data, sink)?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn check_variant_data(
    ctx: &VisibilityContext<'_, '_>,
    type_def_id: LocalDefId,
    variant_data: &VariantData<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    for field in variant_data.fields() {
        check_field(ctx, type_def_id, field, sink)?;
    }
    Ok(())
}

fn check_field(
    ctx: &VisibilityContext<'_, '_>,
    type_def_id: LocalDefId,
    field: &FieldDef<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if field.span.from_expansion() || field.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = source::real_file_path(ctx.tcx, field.vis_span) else {
        return Ok(());
    };
    let Some(field_vis_text) = source::visibility_text(ctx.tcx, field.vis_span)? else {
        return Ok(());
    };
    // A field with no `pub` annotation is private; nothing to narrow.
    if field_vis_text.is_empty() {
        return Ok(());
    }

    let Some(type_vis_text) = type_visibility_text(ctx, type_def_id)? else {
        return Ok(());
    };
    // Only fire on truly-dead cases: the containing type has no `pub`
    // annotation at all (private to its parent module). The conventional
    // pattern of `pub` fields on a `pub(crate)` or `pub(super)` struct is
    // idiomatic Rust shorthand for "as wide as the type allows" — flagging
    // it would push users toward a non-idiomatic style. We only flag when
    // the field's `pub` cannot grant any access at all because the type
    // itself is private.
    if !type_vis_text.is_empty() {
        return Ok(());
    }

    let field_def_id = field.def_id;
    let field_declared = ctx.tcx.visibility(field_def_id.to_def_id());
    let type_declared = ctx.tcx.visibility(type_def_id.to_def_id());

    if !visibility_strictly_wider(ctx.tcx, field_declared, type_declared) {
        return Ok(());
    }
    // Re-export refinement: if the type is `pub use`d to a wider scope, the
    // wider field annotation is honest, not dead.
    let type_effective = effective_type_visibility(ctx, type_def_id);
    if type_effective.is_at_least(field_declared, ctx.tcx) {
        return Ok(());
    }

    let highlight_span = field_highlight_span(field);
    let field_name = field.ident.to_string();
    let finding = source::build_finding(
        ctx.tcx,
        &file_path,
        highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            code:                    DiagnosticCode::FieldVisibilityWiderThanType,
            item:                    Some(format!("field {field_name}")),
            message:                 format_message(&field_vis_text, &type_vis_text),
            suggestion:              Some(suggested_replacement(&type_vis_text)),
            fixability:              FixSupport::FixFieldVisibility,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?;
    sink.findings.push(finding);
    Ok(())
}

/// Effective visibility of the type, accounting for `pub use` re-exports
/// that widen its reach beyond the declared annotation. Falls back to the
/// declared visibility when rustc's effective-visibility table has no
/// entry (typical for items not reachable through any public path).
fn effective_type_visibility(
    ctx: &VisibilityContext<'_, '_>,
    type_def_id: LocalDefId,
) -> DefIdVisibility {
    if let Some(eff) = ctx.effective_visibilities.effective_vis(type_def_id) {
        return eff.at_level(Level::Reachable).to_def_id();
    }
    ctx.tcx.visibility(type_def_id.to_def_id())
}

fn visibility_strictly_wider(tcx: TyCtxt<'_>, lhs: DefIdVisibility, rhs: DefIdVisibility) -> bool {
    lhs.is_at_least(rhs, tcx) && !rhs.is_at_least(lhs, tcx)
}

/// Returns the type's literal visibility annotation text. An empty string
/// means the type has no `pub` annotation (it is private to its parent
/// module) — that is still a valid "narrow target" for the field, since
/// removing the field's `pub` matches the type's privacy.
fn type_visibility_text(
    ctx: &VisibilityContext<'_, '_>,
    type_def_id: LocalDefId,
) -> Result<Option<String>> {
    let item = ctx.tcx.hir_expect_item(type_def_id);
    if item.vis_span.from_expansion() {
        return Ok(None);
    }
    source::visibility_text(ctx.tcx, item.vis_span)
}

/// Span for highlighting the field's vis annotation. Falls back to the
/// field's overall span when the `vis_span` is empty (no annotation).
fn field_highlight_span(field: &FieldDef<'_>) -> Span {
    let vis_span = field.vis_span;
    if vis_span.is_empty() {
        field.span
    } else {
        vis_span.with_hi(field.ident.span.hi())
    }
}

fn format_message(field_vis_text: &str, type_vis_text: &str) -> String {
    let type_label = if type_vis_text.is_empty() {
        "private (no `pub` annotation)".to_string()
    } else {
        format!("`{type_vis_text}`")
    };
    format!(
        "field is declared `{field_vis_text}` but the containing type is \
         {type_label}; the wider field annotation is dead because the \
         type's visibility caps it"
    )
}

fn suggested_replacement(type_vis_text: &str) -> String {
    if type_vis_text.is_empty() {
        "remove the field's visibility annotation".to_string()
    } else {
        format!("consider using: `{type_vis_text}`")
    }
}

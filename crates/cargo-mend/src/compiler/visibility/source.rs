use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rustc_hir::ForeignItemKind;
use rustc_hir::ImplItemKind;
use rustc_hir::ItemKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::LocalDefId;

use super::FindingParams;
use super::source_cache::SourceCache;
use crate::compiler::persistence::StoredFinding;

#[derive(Debug)]
struct LineDisplay {
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
}

pub(super) fn build_finding(
    tcx: TyCtxt<'_>,
    file_path: &Path,
    highlight_span: Span,
    params: FindingParams,
) -> Result<StoredFinding> {
    let display = line_display(tcx, file_path, highlight_span)?;
    Ok(StoredFinding {
        severity:                params.severity,
        code:                    params.code,
        path:                    file_path.to_string_lossy().into_owned(),
        line:                    display.line,
        column:                  display.column,
        highlight_len:           display.highlight_len,
        source_line:             display.source_line,
        item:                    params.item,
        message:                 params.message,
        suggestion:              params.suggestion,
        fixability:              params.fixability,
        related:                 params.related,
        item_def_path:           params.item_def_path,
        narrower_scope_def_path: params.narrower_scope_def_path,
    })
}

pub(super) fn build_line_finding(
    source_cache: &SourceCache,
    file_path: &Path,
    line: usize,
    params: FindingParams,
) -> Result<StoredFinding> {
    let text = source_cache.read_source(file_path)?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();
    let trimmed = source_line.trim_start();
    let column = source_line.len().saturating_sub(trimmed.len()) + 1;
    let highlight_len = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len())
        .max(1);

    Ok(StoredFinding {
        severity: params.severity,
        code: params.code,
        path: file_path.to_string_lossy().into_owned(),
        line,
        column,
        highlight_len,
        source_line,
        item: params.item,
        message: params.message,
        suggestion: params.suggestion,
        fixability: params.fixability,
        related: params.related,
        item_def_path: params.item_def_path,
        narrower_scope_def_path: params.narrower_scope_def_path,
    })
}

pub(super) fn highlight_span(vis_span: Span, ident_span: Option<Span>) -> Span {
    ident_span.map_or(vis_span, |ident_span| vis_span.to(ident_span))
}

pub(super) fn impl_self_type_name_from_tcx(
    tcx: TyCtxt<'_>,
    impl_item_def: LocalDefId,
) -> Option<String> {
    let hir_id = tcx.local_def_id_to_hir_id(impl_item_def);
    let parent_id = tcx.hir_get_parent_item(hir_id);
    let parent_node = tcx.hir_node_by_def_id(parent_id.def_id);
    let rustc_hir::Node::Item(parent_item) = parent_node else {
        return None;
    };
    let ItemKind::Impl(impl_block) = parent_item.kind else {
        return None;
    };
    let rustc_hir::TyKind::Path(rustc_hir::QPath::Resolved(_, path)) = impl_block.self_ty.kind
    else {
        return None;
    };
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(super) const fn item_kind_label(kind: ItemKind<'_>) -> Option<&'static str> {
    match kind {
        ItemKind::Const(..) => Some("const"),
        ItemKind::Enum(..) => Some("enum"),
        ItemKind::Fn { .. } => Some("fn"),
        ItemKind::Static(..) => Some("static"),
        ItemKind::Struct(..) => Some("struct"),
        ItemKind::Trait(..) | ItemKind::TraitAlias(..) => Some("trait"),
        ItemKind::TyAlias(..) => Some("type"),
        ItemKind::Union(..) => Some("union"),
        ItemKind::Mod(..) => Some("mod"),
        ItemKind::Use(..)
        | ItemKind::ExternCrate(..)
        | ItemKind::ForeignMod { .. }
        | ItemKind::GlobalAsm { .. }
        | ItemKind::Impl(..)
        | ItemKind::Macro(..) => None,
    }
}

pub(super) const fn impl_item_kind_label(kind: ImplItemKind<'_>) -> &'static str {
    match kind {
        ImplItemKind::Const(..) => "const",
        ImplItemKind::Fn(..) => "fn",
        ImplItemKind::Type(..) => "type",
    }
}

pub(super) const fn foreign_item_kind_label(kind: ForeignItemKind<'_>) -> &'static str {
    match kind {
        ForeignItemKind::Fn(..) => "fn",
        ForeignItemKind::Static(..) => "static",
        ForeignItemKind::Type => "type",
    }
}

pub(super) fn real_file_path(tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
    let source_map = tcx.sess.source_map();
    let file = source_map.lookup_char_pos(span.lo()).file;
    real_file_path_from_name(file.name.clone())
}

pub(super) fn use_item_contains_glob(tcx: TyCtxt<'_>, span: Span) -> Result<bool> {
    let snippet = tcx.sess.source_map().span_to_snippet(span).map_err(|err| {
        anyhow::anyhow!("failed to extract use item snippet for span {span:?}: {err:?}")
    })?;
    Ok(snippet.contains('*'))
}

pub(super) fn visibility_text(tcx: TyCtxt<'_>, vis_span: Span) -> Result<Option<String>> {
    if vis_span.is_dummy() {
        return Ok(None);
    }
    Ok(Some(
        tcx.sess
            .source_map()
            .span_to_snippet(vis_span)
            .map_err(|err| {
                anyhow::anyhow!(
                    "failed to extract visibility snippet for span {vis_span:?}: {err:?}"
                )
            })?
            .trim()
            .to_string(),
    ))
}

fn line_display(tcx: TyCtxt<'_>, file_path: &Path, span: Span) -> Result<LineDisplay> {
    let source_map = tcx.sess.source_map();
    let start = source_map.lookup_char_pos(span.lo());
    let end = source_map.lookup_char_pos(span.hi());
    let line = start.line;
    let column = start.col_display + 1;
    let highlight_len = if start.line == end.line {
        (end.col_display.saturating_sub(start.col_display)).max(1)
    } else {
        1
    };
    let text = fs::read_to_string(file_path)
        .with_context(|| format!("failed to read source file {}", file_path.display()))?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    Ok(LineDisplay {
        line,
        column,
        highlight_len,
        source_line,
    })
}

fn real_file_path_from_name(name: FileName) -> Option<PathBuf> {
    match name {
        FileName::Real(real) => real.local_path().map(Path::to_path_buf),
        _ => None,
    }
}

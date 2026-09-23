use std::path::Path;

use anyhow::Result;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::UseKind;
use rustc_hir::def::DefKind;
use rustc_hir::def::Res;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::FindingParams;
use crate::compiler::persistence::StoredFinding;
use crate::compiler::visibility::source;
use crate::config::DiagnosticCode;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

/// The module a crate prelude re-exports its parent's items from. `pub use
/// super::Item` there is the prelude's whole purpose, not a sideways facade.
const PRELUDE_MODULE_NAME: &str = "prelude";

/// Flags a re-export whose path leaves the re-exporting module's own subtree.
///
/// A re-export is a facade: the module that owns an item's path hands it
/// outward. Only a module whose subtree holds the item can do that, so the
/// re-export belongs in the parent `mod.rs` (or the crate root). A `pub use
/// super::sibling::item` or `pub use crate::elsewhere::item` instead makes one
/// module a back door into another, and callers reach the item under a module
/// that does not own it.
pub(super) fn finding(
    tcx: TyCtxt<'_>,
    item: &Item<'_>,
    file_path: &Path,
) -> Result<Option<StoredFinding>> {
    let ItemKind::Use(path, use_kind) = item.kind else {
        return Ok(None);
    };
    if matches!(use_kind, UseKind::ListStem) {
        return Ok(None);
    }
    let owner_module: LocalDefId = tcx.parent_module_from_def_id(item.owner_id.def_id).into();
    if !is_reexport(tcx, item.owner_id.def_id, owner_module) || is_prelude_module(tcx, owner_module)
    {
        return Ok(None);
    }
    // A single import names an item after its last segment; a glob names the
    // module it opens with every segment.
    let module_segments = match use_kind {
        UseKind::Glob => path.segments,
        _ => path
            .segments
            .split_last()
            .map_or(&[][..], |(_, prefix)| prefix),
    };
    let Some(source_module) = entered_module(tcx, owner_module, module_segments) else {
        return Ok(None);
    };
    if tcx.is_descendant_of(source_module.to_def_id(), owner_module.to_def_id()) {
        return Ok(None);
    }

    let message = format!(
        "`{}` re-exports from `{}`, which is outside its own subtree",
        module_display_path(tcx, owner_module),
        module_display_path(tcx, source_module),
    );
    source::build_finding(
        tcx,
        file_path,
        item.span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::PubUseOutsideSubtree,
            item: None,
            message,
            suggestion: None,
            fix_support: FixSupport::None,
            related: None,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )
    .map(Some)
}

/// A `use` whose reach extends past its own module re-exports what it names.
/// `pub(self) use` and a bare `use` stay private and re-export nothing.
fn is_reexport(tcx: TyCtxt<'_>, use_def_id: LocalDefId, owner_module: LocalDefId) -> bool {
    match tcx.local_visibility(use_def_id) {
        Visibility::Public => true,
        Visibility::Restricted(scope) => scope != owner_module,
    }
}

fn is_prelude_module(tcx: TyCtxt<'_>, module: LocalDefId) -> bool {
    module != CRATE_DEF_ID
        && tcx
            .opt_item_name(module.to_def_id())
            .is_some_and(|name| name.as_str() == PRELUDE_MODULE_NAME)
}

/// Walks `segments` from `owner_module` and returns the last local module the
/// path passes through. `None` when the path starts outside the crate (an
/// extern crate or an unresolvable first segment), since a re-export of
/// another crate's item has no subtree here to leave.
fn entered_module(
    tcx: TyCtxt<'_>,
    owner_module: LocalDefId,
    segments: &[rustc_hir::PathSegment<'_>],
) -> Option<LocalDefId> {
    let mut module = owner_module;
    for (segment_index, segment) in segments.iter().enumerate() {
        match segment.ident.name.as_str() {
            "self" => {},
            "super" => module = tcx.parent_module_from_def_id(module).into(),
            "crate" if segment_index == 0 => module = CRATE_DEF_ID,
            _ => {
                let child = tcx
                    .module_children_local(module)
                    .iter()
                    .find(|child| child.ident.name == segment.ident.name);
                match child.map(|child| child.res) {
                    Some(Res::Def(DefKind::Mod, def_id)) => module = def_id.as_local()?,
                    None if segment_index == 0 => return None,
                    // An enum or trait on the way to the item, or a name the
                    // module does not list: the path has entered its last
                    // module already.
                    _ => return Some(module),
                }
            },
        }
    }
    Some(module)
}

fn module_display_path(tcx: TyCtxt<'_>, module: LocalDefId) -> String {
    if module == CRATE_DEF_ID {
        "crate".to_string()
    } else {
        format!("crate::{}", tcx.def_path_str(module.to_def_id()))
    }
}

use std::path::Path;

use anyhow::Result;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::PathSegment;
use rustc_hir::UseKind;
use rustc_hir::def::DefKind;
use rustc_hir::def::Res;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::FindingParams;
use super::subtree_reexport_fix;
use super::subtree_reexport_fix::PathWalk;
use super::subtree_reexport_fix::SubtreeReexport;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::visibility::source;
use crate::config::DiagnosticCode;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;
use crate::reporting::PUB_USE_OUTSIDE_SUBTREE_HELP;
use crate::reporting::Severity;

/// The module a crate prelude re-exports its parent's items from. `pub use
/// super::Item` there is the prelude's whole purpose, not a sideways facade.
const PRELUDE_MODULE_NAME: &str = "prelude";

/// The last local module a re-export's path enters, and whether the walk
/// reached it through module segments only.
struct EnteredModule {
    module:    LocalDefId,
    path_walk: PathWalk,
}

/// Flags a re-export whose path leaves the re-exporting module's own subtree.
///
/// A re-export is a facade: the module that owns an item's path hands it
/// outward. Only a module whose subtree holds the item can do that, so the
/// re-export belongs in the parent `mod.rs` (or the crate root). A `pub use
/// super::sibling::item` or `pub use crate::elsewhere::item` instead makes one
/// module a back door into another, and callers reach the item under a module
/// that does not own it.
///
/// Records the finding in `sink`, and for a re-export `--fix` can rewrite a
/// `StoredSubtreeReexportFixFact` at the finding's position; otherwise the
/// finding's help names why no automatic fix applies.
pub(super) fn record(
    tcx: TyCtxt<'_>,
    item: &Item<'_>,
    file_path: &Path,
    crate_root_file: &Path,
    sink: &mut FindingsSink,
) -> Result<()> {
    // A `use` a macro wrote has no source line of its own to rewrite.
    if item.span.from_expansion() {
        return Ok(());
    }
    let ItemKind::Use(path, use_kind) = item.kind else {
        return Ok(());
    };
    if matches!(use_kind, UseKind::ListStem) {
        return Ok(());
    }
    let owner_module: LocalDefId = tcx.parent_module_from_def_id(item.owner_id.def_id).into();
    if !is_reexport(tcx, item.owner_id.def_id, owner_module) || is_prelude_module(tcx, owner_module)
    {
        return Ok(());
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
    let Some(entered) = entered_module(tcx, owner_module, module_segments) else {
        return Ok(());
    };
    let source_module = entered.module;
    if tcx.is_descendant_of(source_module.to_def_id(), owner_module.to_def_id()) {
        return Ok(());
    }

    let message = format!(
        "`{}` re-exports from `{}`, which is outside its own subtree",
        module_display_path(tcx, owner_module),
        module_display_path(tcx, source_module),
    );
    let classification = subtree_reexport_fix::classify(
        tcx,
        &SubtreeReexport {
            item,
            path,
            use_kind,
            owner_module,
            source_module,
            path_walk: entered.path_walk,
        },
        crate_root_file,
    );
    let (fix_support, suggestion) = match &classification {
        Ok(_) => (FixSupport::PubUseOutsideSubtree, None),
        Err(no_fix_reason) => (
            FixSupport::None,
            Some(format!(
                "{PUB_USE_OUTSIDE_SUBTREE_HELP} (no automatic fix: {no_fix_reason})"
            )),
        ),
    };
    let finding = source::build_finding(
        tcx,
        file_path,
        item.span,
        FindingParams {
            severity: Severity::Error,
            diagnostic_code: DiagnosticCode::PubUseOutsideSubtree,
            item: None,
            message,
            suggestion,
            fix_support,
            related: None,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        },
    )?;
    if let Ok(subtree_reexport_fix) = classification {
        sink.subtree_reexport_fix_facts
            .push(subtree_reexport_fix.into_fact(&finding, crate_root_file));
    }
    sink.findings.push(finding);
    Ok(())
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
    segments: &[PathSegment<'_>],
) -> Option<EnteredModule> {
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
                    _ => {
                        return Some(EnteredModule {
                            module,
                            path_walk: PathWalk::Interrupted,
                        });
                    },
                }
            },
        }
    }
    Some(EnteredModule {
        module,
        path_walk: PathWalk::Complete,
    })
}

pub(super) fn module_display_path(tcx: TyCtxt<'_>, module: LocalDefId) -> String {
    if module == CRATE_DEF_ID {
        "crate".to_string()
    } else {
        format!("crate::{}", tcx.def_path_str(module.to_def_id()))
    }
}

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use rustc_middle::ty::TyCtxt;
use rustc_span::ExpnKind;
use rustc_span::MacroKind;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::sym;

use super::subtree_reexport_fix;
use crate::compiler::persistence::StoredModuleMountFact;
use crate::compiler::visibility::source;

/// Records the files the `--fix` for `pub_use_outside_subtree` cannot find by
/// following `mod` declarations from the crate root: a file whose items an
/// `include!` places directly in a module, and a module file outside the crate
/// root file's directory. Each is paired with the module its items belong to.
pub(super) fn collect_module_mounts(
    tcx: TyCtxt<'_>,
    crate_root_file: &Path,
) -> Vec<StoredModuleMountFact> {
    let crate_root_directory = crate_root_file.parent();
    let mut mounts: BTreeSet<(PathBuf, Vec<String>)> = BTreeSet::new();
    tcx.hir_for_each_module(|module| {
        let module_def_id = module.to_local_def_id();
        let Ok(module_path) = subtree_reexport_fix::module_path(tcx, module_def_id) else {
            return;
        };
        let (hir_module, _, _) = tcx.hir_get_module(module);
        let module_file = if module_def_id == CRATE_DEF_ID {
            Some(crate_root_file.to_path_buf())
        } else {
            source::real_file_path(tcx, hir_module.spans.inner_span)
        };
        let Some(module_file) = module_file else {
            return;
        };
        for item in hir_module
            .item_ids
            .iter()
            .map(|item_id| tcx.hir_item(*item_id))
        {
            if !is_written_or_included(item.span) {
                continue;
            }
            if let Some(item_file) = source::real_file_path(tcx, item.span)
                && item_file != module_file
            {
                mounts.insert((item_file, module_path.clone()));
            }
        }
        if !crate_root_directory.is_some_and(|directory| module_file.starts_with(directory)) {
            mounts.insert((module_file, module_path));
        }
    });
    mounts
        .into_iter()
        .map(|(file, module_path)| StoredModuleMountFact {
            crate_root_file: crate_root_file.to_string_lossy().into_owned(),
            file: file.to_string_lossy().into_owned(),
            module_path,
        })
        .collect()
}

/// Items written in a source file, or placed in the module by `include!`.
fn is_written_or_included(span: Span) -> bool {
    !span.from_expansion()
        || span.ctxt().outer_expn_data().kind == ExpnKind::Macro(MacroKind::Bang, sym::include)
}

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

/// Records every compiled source mount, whether or not it emitted a diagnostic.
/// Inline modules in the same file are not additional mounts of that file.
pub(super) fn collect_module_mounts(
    tcx: TyCtxt<'_>,
    crate_root_file: &Path,
) -> Vec<StoredModuleMountFact> {
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
            match item_source_origin(item.span) {
                ItemSourceOrigin::WrittenOrIncluded => {},
                ItemSourceOrigin::OtherExpansion => continue,
            }
            if let Some(item_file) = source::real_file_path(tcx, item.span)
                && item_file != module_file
            {
                mounts.insert((item_file, module_path.clone()));
            }
        }
        let declaration_file = source::real_file_path(tcx, tcx.def_span(module_def_id));
        if module_def_id == CRATE_DEF_ID || declaration_file.as_ref() != Some(&module_file) {
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

/// Only written items and `include!` items have source files to record as module mounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemSourceOrigin {
    WrittenOrIncluded,
    OtherExpansion,
}

impl ItemSourceOrigin {
    fn for_expansion(kind: &ExpnKind) -> Self {
        if *kind == ExpnKind::Macro(MacroKind::Bang, sym::include) {
            Self::WrittenOrIncluded
        } else {
            Self::OtherExpansion
        }
    }
}

fn item_source_origin(span: Span) -> ItemSourceOrigin {
    if span.from_expansion() {
        ItemSourceOrigin::for_expansion(&span.ctxt().outer_expn_data().kind)
    } else {
        ItemSourceOrigin::WrittenOrIncluded
    }
}

#[cfg(test)]
mod tests {
    use rustc_span::DUMMY_SP;
    use rustc_span::ExpnKind;
    use rustc_span::MacroKind;
    use rustc_span::create_default_session_globals_then;
    use rustc_span::sym;

    use super::ItemSourceOrigin;
    use super::item_source_origin;

    #[test]
    fn module_mounts_keep_written_and_included_items_only() {
        create_default_session_globals_then(|| {
            assert_eq!(
                item_source_origin(DUMMY_SP),
                ItemSourceOrigin::WrittenOrIncluded
            );
            for (kind, expected) in [
                (
                    ExpnKind::Macro(MacroKind::Bang, sym::include),
                    ItemSourceOrigin::WrittenOrIncluded,
                ),
                (
                    ExpnKind::Macro(MacroKind::Bang, sym::stringify),
                    ItemSourceOrigin::OtherExpansion,
                ),
                (
                    ExpnKind::Macro(MacroKind::Attr, sym::include),
                    ItemSourceOrigin::OtherExpansion,
                ),
                (
                    ExpnKind::Macro(MacroKind::Derive, sym::include),
                    ItemSourceOrigin::OtherExpansion,
                ),
            ] {
                assert_eq!(ItemSourceOrigin::for_expansion(&kind), expected);
            }
        });
    }
}

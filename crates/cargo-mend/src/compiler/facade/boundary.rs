use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use crate::compiler::source_cache::SourceCache;

#[derive(Debug, Clone)]
pub struct ParentBoundary {
    pub boundary_file: PathBuf,
    pub module_path:   Vec<String>,
}

pub struct LogicalParentBoundary {
    pub module:      LocalDefId,
    pub module_path: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ModuleSourceMap {
    modules_by_file:            HashMap<PathBuf, Vec<LocalDefId>>,
    files_by_module:            HashMap<LocalDefId, Vec<PathBuf>>,
    modules_by_path:            HashMap<Vec<String>, LocalDefId>,
    structural_parents_by_file: HashMap<PathBuf, Vec<Vec<String>>>,
    crate_files:                HashSet<PathBuf>,
}

impl ModuleSourceMap {
    pub fn new(tcx: TyCtxt<'_>, source_cache: &SourceCache) -> Self {
        let mut module_sources = Self {
            structural_parents_by_file: source_cache.structural_parent_module_paths().clone(),
            crate_files: source_cache
                .source_files()
                .into_iter()
                .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
                .collect(),
            ..Self::default()
        };
        module_sources
            .modules_by_path
            .insert(Vec::new(), CRATE_DEF_ID);
        for item_id in tcx.hir_crate_items(()).free_items() {
            let item = tcx.hir_item(item_id);
            let Some(source_file) = real_file_path(tcx, item.span) else {
                continue;
            };
            let module: LocalDefId = tcx.parent_module_from_def_id(item.owner_id.def_id).into();
            module_sources.insert(module, source_file);
        }
        tcx.hir_for_each_module(|module| {
            let module_def_id = module.to_local_def_id();
            module_sources
                .modules_by_path
                .insert(module_path(tcx, module_def_id), module_def_id);
            let (hir_module, _, _) = tcx.hir_get_module(module);
            if let Some(source_file) = real_file_path(tcx, hir_module.spans.inner_span) {
                module_sources.insert(module_def_id, source_file);
            }
        });
        module_sources
    }

    fn insert(&mut self, module: LocalDefId, source_file: PathBuf) {
        let modules = self.modules_by_file.entry(source_file.clone()).or_default();
        if !modules.contains(&module) {
            modules.push(module);
        }
        let files = self.files_by_module.entry(module).or_default();
        if !files.contains(&source_file) {
            files.push(source_file);
        }
    }

    pub fn root_modules_for_file(&self, tcx: TyCtxt<'_>, source_file: &Path) -> Vec<LocalDefId> {
        let canonical_source_file =
            fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf());
        let root_modules = self
            .modules_by_file
            .get(&canonical_source_file)
            .into_iter()
            .flatten()
            .copied()
            .filter(|module| {
                if *module == CRATE_DEF_ID {
                    return true;
                }
                let parent: LocalDefId = tcx.parent_module_from_def_id(*module).into();
                self.files_by_module
                    .get(&parent)
                    .is_none_or(|files| !files.contains(&canonical_source_file))
            })
            .collect::<Vec<_>>();
        if !root_modules.is_empty() || !self.crate_files.contains(&canonical_source_file) {
            return root_modules;
        }
        let mut structural_roots = Vec::new();
        for parent_path in self
            .structural_parents_by_file
            .get(&canonical_source_file)
            .into_iter()
            .flatten()
        {
            let module = self.nearest_active_ancestor(parent_path);
            if !structural_roots.contains(&module) {
                structural_roots.push(module);
            }
        }
        if structural_roots.is_empty() {
            vec![CRATE_DEF_ID]
        } else {
            structural_roots
        }
    }

    fn nearest_active_ancestor(&self, module_path: &[String]) -> LocalDefId {
        for path_length in (0..=module_path.len()).rev() {
            if let Some(module) = self.modules_by_path.get(&module_path[..path_length]) {
                return *module;
            }
        }
        CRATE_DEF_ID
    }

    pub fn source_files(&self, module: LocalDefId) -> &[PathBuf] {
        self.files_by_module.get(&module).map_or(&[], Vec::as_slice)
    }

    pub fn file_contains_module_path(
        &self,
        tcx: TyCtxt<'_>,
        source_file: &Path,
        expected: &[String],
    ) -> bool {
        let canonical_source_file =
            fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf());
        self.modules_by_file
            .get(&canonical_source_file)
            .is_some_and(|modules| {
                modules
                    .iter()
                    .any(|module| module_path(tcx, *module) == expected)
            })
    }
}

/// Locate a facade's owning module from the compiler-resolved `use` item.
///
/// This path is intentionally used only after HIR has selected a re-export.
/// The source path supplies reporting and usage-analysis metadata; it never
/// decides whether a facade exists.
pub fn parent_boundary_for_reexport(
    tcx: TyCtxt<'_>,
    owner_module: LocalDefId,
    use_span: Span,
) -> Option<ParentBoundary> {
    let boundary_file = real_file_path(tcx, use_span)?;
    let module_path = if owner_module == CRATE_DEF_ID {
        Vec::new()
    } else {
        tcx.def_path_str(owner_module.to_def_id())
            .split("::")
            .filter(|segment| !segment.is_empty())
            .map(String::from)
            .collect()
    };
    Some(ParentBoundary {
        boundary_file,
        module_path,
    })
}

pub fn logical_parent_boundary_for_child(
    tcx: TyCtxt<'_>,
    child_item: LocalDefId,
) -> Option<LogicalParentBoundary> {
    let child_module: LocalDefId = tcx.parent_module_from_def_id(child_item).into();
    if child_module == CRATE_DEF_ID {
        return None;
    }
    let module: LocalDefId = tcx.parent_module_from_def_id(child_module).into();
    Some(LogicalParentBoundary {
        module,
        module_path: module_path(tcx, module),
    })
}

pub fn module_path(tcx: TyCtxt<'_>, module: LocalDefId) -> Vec<String> {
    if module == CRATE_DEF_ID {
        return Vec::new();
    }
    tcx.def_path_str(module.to_def_id())
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(String::from)
        .collect()
}

pub fn module_is_within(tcx: TyCtxt<'_>, mut candidate: LocalDefId, ancestor: LocalDefId) -> bool {
    loop {
        if candidate == ancestor {
            return true;
        }
        if candidate == CRATE_DEF_ID {
            return false;
        }
        candidate = tcx.parent_module_from_def_id(candidate).into();
    }
}

fn real_file_path(tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
    let source_map = tcx.sess.source_map();
    let file = source_map.lookup_char_pos(span.lo()).file;
    match file.name.clone() {
        FileName::Real(real) => real
            .local_path()
            .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())),
        _ => None,
    }
}

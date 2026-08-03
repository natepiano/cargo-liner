use std::borrow::Cow;
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

pub(in crate::compiler) struct LogicalParentBoundary {
    module:      LocalDefId,
    module_path: Vec<String>,
}

impl LogicalParentBoundary {
    pub(in crate::compiler) const fn module(&self) -> LocalDefId { self.module }

    pub(in crate::compiler) fn module_path(&self) -> &[String] { &self.module_path }
}

#[derive(Debug, Default)]
pub struct ModuleSourceMap {
    modules_by_file:            HashMap<PathBuf, Vec<LocalDefId>>,
    files_by_module:            HashMap<LocalDefId, Vec<PathBuf>>,
    modules_by_path:            HashMap<Vec<String>, LocalDefId>,
    structural_parents_by_file: HashMap<PathBuf, Vec<Vec<String>>>,
    crate_files:                HashSet<PathBuf>,
    canonical_by_source_file:   HashMap<PathBuf, PathBuf>,
    canonical_by_span_file:     HashMap<PathBuf, PathBuf>,
}

impl ModuleSourceMap {
    pub fn new(tcx: TyCtxt<'_>, source_cache: &SourceCache) -> Self {
        let canonical_by_source_file: HashMap<PathBuf, PathBuf> = source_cache
            .source_files()
            .into_iter()
            .map(|path| {
                let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                (path.to_path_buf(), canonical)
            })
            .collect();
        let mut module_sources = Self {
            structural_parents_by_file: source_cache.structural_parent_module_paths().clone(),
            crate_files: canonical_by_source_file.values().cloned().collect(),
            canonical_by_source_file,
            canonical_by_span_file: canonical_span_files(tcx),
            ..Self::default()
        };
        module_sources
            .modules_by_path
            .insert(Vec::new(), CRATE_DEF_ID);
        for item_id in tcx.hir_crate_items(()).free_items() {
            let item = tcx.hir_item(item_id);
            let Some(source_file) = module_sources.canonical_span_file(tcx, item.span) else {
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
            if let Some(source_file) =
                module_sources.canonical_span_file(tcx, hir_module.spans.inner_span)
            {
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

    /// Canonical form of `source_file`, taken from the table built in `new`
    /// when the path came from the `SourceCache`.
    ///
    /// The exposure walk calls `root_modules_for_file` once per evaluated item
    /// per source file, so canonicalizing there runs `realpath` — one
    /// `getattrlist` syscall per path component — in the innermost loop. The
    /// declaration searches in `exposure::detect` sit in the same loop and
    /// canonicalize the same paths, so they share this table.
    pub fn canonical_source_file<'path>(&'path self, source_file: &'path Path) -> Cow<'path, Path> {
        self.canonical_by_source_file.get(source_file).map_or_else(
            || {
                Cow::Owned(
                    fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf()),
                )
            },
            |canonical| Cow::Borrowed(canonical.as_path()),
        )
    }

    /// Canonical path of the file `span` originates in, or `None` for a span
    /// with no real file behind it.
    ///
    /// Resolving a span's file is otherwise a `realpath` call per span, and the
    /// exposure walk resolves one span per candidate declaration and one per
    /// struct field. Spans in a file all report the same path, so the table
    /// built in `new` answers from memory; a file that entered the source map
    /// after that (external crate sources are loaded on demand) falls back to
    /// the syscall.
    pub fn canonical_span_file(&self, tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
        let file = tcx.sess.source_map().lookup_char_pos(span.lo()).file;
        let FileName::Real(real) = &file.name else {
            return None;
        };
        let local_path = real.local_path()?;
        Some(
            self.canonical_by_span_file
                .get(local_path)
                .cloned()
                .unwrap_or_else(|| {
                    fs::canonicalize(local_path).unwrap_or_else(|_| local_path.to_path_buf())
                }),
        )
    }

    /// Whether `span` originates in `canonical_file`.
    ///
    /// `canonical_file` must already be canonical: `canonical_span_file`
    /// canonicalizes what it returns, so an uncanonical argument never compares
    /// equal.
    pub fn span_is_in_file(&self, tcx: TyCtxt<'_>, span: Span, canonical_file: &Path) -> bool {
        self.canonical_span_file(tcx, span)
            .is_some_and(|file| file == canonical_file)
    }

    pub fn root_modules_for_file(&self, tcx: TyCtxt<'_>, source_file: &Path) -> Vec<LocalDefId> {
        let canonical = self.canonical_source_file(source_file);
        let canonical_source_file: &Path = canonical.as_ref();
        let root_modules = self
            .modules_by_file
            .get(canonical_source_file)
            .into_iter()
            .flatten()
            .copied()
            .filter(|module| {
                if *module == CRATE_DEF_ID {
                    return true;
                }
                let parent: LocalDefId = tcx.parent_module_from_def_id(*module).into();
                self.files_by_module.get(&parent).is_none_or(|files| {
                    !files
                        .iter()
                        .any(|file| file.as_path() == canonical_source_file)
                })
            })
            .collect::<Vec<_>>();
        if !root_modules.is_empty() || !self.crate_files.contains(canonical_source_file) {
            return root_modules;
        }
        let mut structural_roots = Vec::new();
        for parent_path in self
            .structural_parents_by_file
            .get(canonical_source_file)
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
        let canonical_source_file = self.canonical_source_file(source_file);
        self.modules_by_file
            .get(canonical_source_file.as_ref())
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

pub(super) fn logical_parent_boundary_for_child(
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

/// Canonical form of every real file the compiler has loaded, keyed by the path
/// the compiler reports for it.
///
/// Canonicalizing once per file here replaces canonicalizing once per span
/// resolved during the exposure walk, which is many thousands of `realpath`
/// calls over the same few hundred paths.
fn canonical_span_files(tcx: TyCtxt<'_>) -> HashMap<PathBuf, PathBuf> {
    tcx.sess
        .source_map()
        .files()
        .iter()
        .filter_map(|file| match &file.name {
            FileName::Real(real) => real.local_path().map(Path::to_path_buf),
            _ => None,
        })
        .map(|path| {
            let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            (path, canonical)
        })
        .collect()
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

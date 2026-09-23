use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::rust_syntax::FileModulePath;
use crate::rust_syntax::ModuleMap;

/// The source files of one crate root, keyed by canonical path, each with the
/// module path it occupies — `None` when `#[path]` attaches it to several
/// parents and no single module path describes it.
#[derive(Default)]
pub(super) struct CrateFiles {
    modules:   BTreeMap<PathBuf, Option<Vec<String>>>,
    /// The files that compile only under test: `#[cfg(test)]` sits on their
    /// `mod` declaration or an ancestor's.
    test_only: BTreeSet<PathBuf>,
}

impl CrateFiles {
    /// The files `mod` declarations reach from `crate_root`, plus the `mounts`
    /// the compiler recorded (`include!` targets and module files outside the
    /// crate root's directory). A declared file keeps its declared module path.
    pub(super) fn resolve(
        crate_root: &Path,
        mounts: impl IntoIterator<Item = (PathBuf, Vec<String>)>,
    ) -> Self {
        let module_map = ModuleMap::for_crate_root(crate_root);
        let mut modules = BTreeMap::new();
        let mut test_only = BTreeSet::new();
        for (file, module_path) in module_map.declared_files() {
            let module_path = match module_path {
                FileModulePath::Known(module_path) => Some(module_path),
                FileModulePath::SeveralParents => None,
            };
            if module_map.is_test_only(file) {
                test_only.insert(canonical(file));
            }
            modules.insert(canonical(file), module_path);
        }
        for (file, module_path) in mounts {
            modules.entry(canonical(&file)).or_insert(Some(module_path));
        }
        Self { modules, test_only }
    }

    /// Every file with a single module path.
    pub(super) fn scannable(&self) -> impl Iterator<Item = (&Path, &[String])> {
        self.modules.iter().filter_map(|(file, module_path)| {
            module_path
                .as_deref()
                .map(|module_path| (file.as_path(), module_path))
        })
    }

    /// Whether `file` (canonical) compiles only under test.
    pub(super) fn is_test_only(&self, file: &Path) -> bool { self.test_only.contains(file) }

    /// Whether `file` (canonical) is attached to several parents.
    pub(super) fn has_several_parents(&self, file: &Path) -> bool {
        matches!(self.modules.get(file), Some(None))
    }
}

/// `path` with symlinks and `..` resolved, so a path the module walk built and
/// one joined from a report fact compare equal; `path` itself when it cannot
/// be resolved.
pub(super) fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

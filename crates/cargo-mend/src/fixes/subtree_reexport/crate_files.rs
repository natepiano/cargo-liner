use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::rust_syntax::FileModulePath;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;

/// The source files of one crate root, keyed by canonical path, each with the
/// module path it occupies, or `SeveralParents` when no single path describes it.
#[derive(Default)]
pub(super) struct CrateFiles {
    modules:   BTreeMap<PathBuf, FileModulePath>,
    /// The files that compile only under test: `#[cfg(test)]` sits on their
    /// `mod` declaration or an ancestor's.
    test_only: BTreeSet<PathBuf>,
}

/// A file is `TestOnly` when `#[cfg(test)]` sits on its module declaration or an ancestor's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileBuilds {
    Every,
    TestOnly,
}

/// Whether a canonical file has one declared module path, several parents, or no declaration.
pub(super) enum ModuleLookup {
    Known,
    SeveralParents,
    Undeclared,
}

impl From<Option<&FileModulePath>> for ModuleLookup {
    fn from(module_path: Option<&FileModulePath>) -> Self {
        match module_path {
            Some(FileModulePath::Known(_)) => Self::Known,
            Some(FileModulePath::SeveralParents) => Self::SeveralParents,
            None => Self::Undeclared,
        }
    }
}

impl CrateFiles {
    /// The files `mod` declarations reach from `crate_root`, plus the `mounts`
    /// the compiler recorded (`include!` targets and module files outside the
    /// crate root's directory). A declared file keeps its declared module path.
    pub(super) fn resolve(
        crate_root: &Path,
        mounts: impl IntoIterator<Item = (PathBuf, Vec<String>)>,
        sources: &ParsedSources,
    ) -> Self {
        let module_map = ModuleMap::for_crate_root(crate_root, sources);
        let mut modules = BTreeMap::new();
        let mut test_only = BTreeSet::new();
        for (file, module_path) in module_map.declared_files() {
            if module_map.is_test_only(file) {
                test_only.insert(canonical(file));
            }
            modules.insert(canonical(file), module_path);
        }
        for (file, module_path) in mounts {
            modules
                .entry(canonical(&file))
                .or_insert(FileModulePath::Known(module_path));
        }
        Self { modules, test_only }
    }

    /// Every file with a single module path.
    pub(super) fn scannable(&self) -> impl Iterator<Item = (&Path, &[String])> {
        self.modules
            .iter()
            .filter_map(|(file, module_path)| match module_path {
                FileModulePath::Known(path) => Some((file.as_path(), path.as_slice())),
                FileModulePath::SeveralParents => None,
            })
    }

    /// The builds that compile `file` (canonical).
    pub(super) fn builds(&self, file: &Path) -> FileBuilds {
        if self.test_only.contains(file) {
            FileBuilds::TestOnly
        } else {
            FileBuilds::Every
        }
    }

    /// The module attachment status of `file` (canonical), classified from the existing map.
    pub(super) fn module_lookup(&self, file: &Path) -> ModuleLookup {
        ModuleLookup::from(self.modules.get(file))
    }
}

/// The crate an owner module sits in: its files, and the sources they are
/// read through.
#[derive(Clone, Copy)]
pub(super) struct OwnerCrate<'a> {
    pub(super) files:   &'a CrateFiles,
    pub(super) sources: &'a ParsedSources,
}

/// `path` with symlinks and `..` resolved, so a path the module walk built and
/// one joined from a report fact compare equal; `path` itself when it cannot
/// be resolved.
pub(super) fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::canonical;
    use crate::rust_syntax::ModuleMap;
    use crate::rust_syntax::ParsedSources;

    #[test]
    fn unobserved_package_reaches_includes_through_external_modules() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path();
        fs::create_dir_all(root.join("b/src"))?;
        fs::write(
            root.join("b/src/lib.rs"),
            "#[path = \"../../wrapper.rs\"] mod gateway;\n",
        )?;
        fs::write(root.join("wrapper.rs"), "include!(\"shared.rs\");\n")?;
        fs::write(
            root.join("shared.rs"),
            "pub use crate::platform::discover;\n",
        )?;
        fs::write(root.join("unrelated.rs"), "pub fn unrelated() {}\n")?;

        let reached =
            ModuleMap::reachable_sources([root.join("b/src/lib.rs")], &ParsedSources::default())?;

        assert_eq!(
            reached,
            ["b/src/lib.rs", "wrapper.rs", "shared.rs"]
                .map(|file| canonical(&root.join(file)))
                .into_iter()
                .collect()
        );
        Ok(())
    }

    #[test]
    fn computed_include_does_not_prove_source_unreachable() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path().join("lib.rs");
        fs::write(
            &root,
            "include!(concat!(env!(\"OUT_DIR\"), \"/shared.rs\"));\n",
        )?;

        assert!(ModuleMap::reachable_sources([root], &ParsedSources::default()).is_err());
        Ok(())
    }
}

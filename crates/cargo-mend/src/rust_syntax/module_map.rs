use std::ffi::OsStr;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use syn::Attribute;
use syn::Expr;
use syn::Item;
use syn::ItemMod;
use syn::Lit;
use syn::Meta;
use syn::ext::IdentExt;
use syn::parse_file;

use super::file_module_path;
use super::is_cfg_test;
use super::parse_meta_list;

/// Where a source file sits in its crate's module tree.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FileModulePath {
    /// The file is declared once, at this module path.
    Known(Vec<String>),
    /// `#[path]` attached the file to more than one parent, so `super` names a
    /// different module in each copy. No suggestion written relative to the
    /// current module is correct for all of them.
    SeveralParents,
}

/// The directories a `mod` declaration resolves its file against.
///
/// The two differ: a `#[path]` written at the top level of a source file is
/// relative to the directory holding that file, while a plain `mod name;` is
/// relative to the module's own directory — the same directory for `mod.rs`,
/// `lib.rs`, and `main.rs`, but `<dir>/<stem>` for every other file.
#[derive(Clone)]
pub(crate) struct ModuleDirectories {
    file:   PathBuf,
    module: PathBuf,
}

impl ModuleDirectories {
    /// The bases a `mod` written at the top level of `source_file` resolves against.
    pub(crate) fn for_file(source_file: &Path) -> Self {
        let directory = source_file
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        let module = match source_file.file_name().and_then(OsStr::to_str) {
            Some("mod.rs" | "lib.rs" | "main.rs") => directory.clone(),
            _ => source_file.with_extension(""),
        };
        Self {
            file: directory,
            module,
        }
    }

    /// The bases a `mod` written at the top level of crate root `crate_root`
    /// resolves against.
    ///
    /// Both are the root file's directory, whatever the file is named: rustc
    /// resolves `mod helper;` in `src/bin/tool.rs` to `src/bin/helper.rs`, as
    /// it would from a `mod.rs`.
    fn for_crate_root(crate_root: &Path) -> Self {
        let directory = crate_root
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        Self {
            file:   directory.clone(),
            module: directory,
        }
    }

    /// The bases a `mod` written inside inline module `name` resolves against.
    ///
    /// Both collapse onto the inline module's directory: inside a `mod` block
    /// even a `#[path]` is relative to the module directory, inline components
    /// included.
    pub(crate) fn inside_inline_module(&self, name: &str) -> Self {
        let module = self.module.join(name);
        Self {
            file: module.clone(),
            module,
        }
    }

    /// Every file `declaration` could name, most specific first.
    ///
    /// `#[path]` wins over the directory default; `#[cfg_attr(_, path = ...)]`
    /// adds a candidate per configuration, since this walk does not evaluate
    /// `cfg` predicates. Candidates that do not exist are the caller's to drop.
    pub(crate) fn declared_module_files(&self, declaration: &ItemMod) -> Vec<PathBuf> {
        let module_name = declaration.ident.unraw().to_string();
        let direct = declaration
            .attrs
            .iter()
            .filter_map(direct_path_attribute)
            .map(|path| self.file.join(path))
            .collect::<Vec<_>>();
        let mut candidates = if direct.is_empty() {
            vec![
                self.module.join(format!("{module_name}.rs")),
                self.module.join(module_name).join("mod.rs"),
            ]
        } else {
            direct
        };
        for path in declaration
            .attrs
            .iter()
            .flat_map(conditional_path_attributes)
        {
            let candidate = self.file.join(path);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        candidates
    }
}

/// The module path each source file of a crate occupies, resolved by walking
/// `mod` declarations from the crate roots.
///
/// Directory layout is only the default. `#[path]` attaches a module to a file
/// anywhere on disk, so `src/stream/macos.rs` reached as
/// `#[path = "../stream/macos.rs"] mod camera_stream;` from `src/platform/mod.rs`
/// is `crate::platform::camera_stream` and not `crate::stream::macos`. A
/// suggestion written in terms of `super` or of a crate-absolute path is only
/// correct against the module path the file really occupies; derived from the
/// layout instead, it names a module that does not exist and the crate stops
/// compiling.
pub(crate) struct ModuleMap {
    declarations: FxHashMap<PathBuf, Vec<Vec<String>>>,
    /// The declared files some chain of `mod` declarations reaches with no
    /// `#[cfg(test)]` on any link; every other declared file compiles only
    /// under test.
    ungated:      FxHashSet<PathBuf>,
}

impl ModuleMap {
    /// Resolve every file reachable from a crate root under `source_root`.
    pub(crate) fn resolve(source_root: &Path) -> Self {
        let mut merged = Self {
            declarations: FxHashMap::default(),
            ungated:      FxHashSet::default(),
        };
        for crate_root in crate_root_files(source_root) {
            merged.absorb(Self::for_crate_root(&crate_root));
        }
        merged
    }

    /// Resolve every file reachable from the one crate root `crate_root`,
    /// which may sit anywhere: `src/lib.rs`, `src/bin/tool.rs`, `tests/it.rs`.
    pub(crate) fn for_crate_root(crate_root: &Path) -> Self {
        let mut walk = ModuleWalk {
            declarations: FxHashMap::default(),
            ungated:      FxHashSet::default(),
            visiting:     FxHashSet::default(),
        };
        let root = Declared {
            module_path: Vec::new(),
            test_only:   false,
        };
        walk.declare(crate_root, &root);
        walk.walk_file(
            crate_root,
            &root,
            &ModuleDirectories::for_crate_root(crate_root),
        );
        Self {
            declarations: walk.declarations,
            ungated:      walk.ungated,
        }
    }

    fn absorb(&mut self, other: Self) {
        self.ungated.extend(other.ungated);
        for (file, module_paths) in other.declarations {
            let declared = self.declarations.entry(file).or_default();
            for module_path in module_paths {
                if !declared.contains(&module_path) {
                    declared.push(module_path);
                }
            }
        }
    }

    /// The module path suggestions in `file` may be written against.
    ///
    /// `None` means no single module path describes the file, so no suggestion
    /// relative to the current module is correct for every copy of it.
    pub(crate) fn scannable_module_path(
        &self,
        source_root: &Path,
        file: &Path,
    ) -> Result<Option<Vec<String>>> {
        match self
            .file_module_path(source_root, file)
            .with_context(|| format!("failed to determine module path for {}", file.display()))?
        {
            FileModulePath::Known(module_path) => Ok(Some(module_path)),
            FileModulePath::SeveralParents => Ok(None),
        }
    }

    /// Every file the walk declared, with the module path it occupies. Paths
    /// are lexically normalized, not canonicalized.
    pub(crate) fn declared_files(&self) -> impl Iterator<Item = (&Path, FileModulePath)> {
        self.declarations.iter().filter_map(|(file, module_paths)| {
            Some((file.as_path(), declared_module_path(module_paths)?))
        })
    }

    /// Whether every `mod` declaration chain reaching `file` carries
    /// `#[cfg(test)]` on the file's own declaration or an ancestor's, so the
    /// file compiles only under test. `false` for a file no chain declares.
    pub(crate) fn is_test_only(&self, file: &Path) -> bool {
        let file = lexically_normalized(file);
        self.declarations.contains_key(&file) && !self.ungated.contains(&file)
    }

    /// The module path `file` occupies.
    ///
    /// A file no crate root declares falls back to its directory layout, which
    /// keeps coverage over files this walk cannot reach — one behind a target
    /// declared with a manifest `path`, or one no `mod` declares at all.
    /// `None` means `file` lies outside `source_root`, where even the layout
    /// says nothing.
    pub(crate) fn file_module_path(
        &self,
        source_root: &Path,
        file: &Path,
    ) -> Option<FileModulePath> {
        self.declarations
            .get(&lexically_normalized(file))
            .and_then(|module_paths| declared_module_path(module_paths))
            .or_else(|| file_module_path(source_root, file).map(FileModulePath::Known))
    }
}

/// Where a file the walk declared at `module_paths` sits; `None` when no
/// declaration recorded a path.
fn declared_module_path(module_paths: &[Vec<String>]) -> Option<FileModulePath> {
    match module_paths {
        [module_path] => Some(FileModulePath::Known(module_path.clone())),
        [_, _, ..] => Some(FileModulePath::SeveralParents),
        [] => None,
    }
}

/// A module the walk reached: its path, and whether a `#[cfg(test)]` on its
/// declaration or an ancestor's limits it to test builds.
struct Declared {
    module_path: Vec<String>,
    test_only:   bool,
}

struct ModuleWalk {
    declarations: FxHashMap<PathBuf, Vec<Vec<String>>>,
    ungated:      FxHashSet<PathBuf>,
    /// The `(file, module path)` pairs already being walked, so a `#[path]`
    /// cycle terminates while a file declared at two paths still records both.
    visiting:     FxHashSet<(PathBuf, Vec<String>)>,
}

impl ModuleWalk {
    fn declare(&mut self, file: &Path, module: &Declared) {
        let file = lexically_normalized(file);
        if !module.test_only {
            self.ungated.insert(file.clone());
        }
        let declared = self.declarations.entry(file).or_default();
        if !declared.contains(&module.module_path) {
            declared.push(module.module_path.clone());
        }
    }

    fn walk_file(&mut self, file: &Path, module: &Declared, directories: &ModuleDirectories) {
        let visit = (lexically_normalized(file), module.module_path.clone());
        if !self.visiting.insert(visit.clone()) {
            return;
        }
        if let Ok(text) = fs::read_to_string(file)
            && let Ok(syntax) = parse_file(&text)
        {
            self.walk_items(&syntax.items, directories, module);
        }
        self.visiting.remove(&visit);
    }

    fn walk_items(&mut self, items: &[Item], directories: &ModuleDirectories, module: &Declared) {
        for item in items {
            let Item::Mod(declaration) = item else {
                continue;
            };
            let module_name = declaration.ident.unraw().to_string();
            let mut module_path = module.module_path.clone();
            module_path.push(module_name.clone());
            let child = Declared {
                module_path,
                test_only: module.test_only || is_cfg_test(&declaration.attrs),
            };

            if let Some((_, inline_items)) = &declaration.content {
                self.walk_items(
                    inline_items,
                    &directories.inside_inline_module(&module_name),
                    &child,
                );
                continue;
            }

            for module_file in directories.declared_module_files(declaration) {
                if !module_file.is_file() {
                    continue;
                }
                self.declare(&module_file, &child);
                self.walk_file(
                    &module_file,
                    &child,
                    &ModuleDirectories::for_file(&module_file),
                );
            }
        }
    }
}

/// Every file cargo compiles as a crate root under `source_root`.
fn crate_root_files(source_root: &Path) -> Vec<PathBuf> {
    let named_roots = ["lib.rs", "main.rs"]
        .into_iter()
        .map(|name| source_root.join(name));
    let binary_roots = fs::read_dir(source_root.join("bin"))
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            [path.join("main.rs"), path]
        });
    named_roots
        .chain(binary_roots)
        .filter(|path| path.extension().and_then(OsStr::to_str) == Some("rs") && path.is_file())
        .collect()
}

/// `path` with `.` and `..` components resolved without touching the filesystem.
///
/// A `#[path]` reaching out of its directory produces
/// `src/platform/../screen/capture_stream.rs`, which names the same file as
/// `src/screen/capture_stream.rs` but does not compare equal to it. Canonicalizing
/// would also resolve symlinks, and the callers' paths come from a directory walk
/// that does not.
fn lexically_normalized(path: &Path) -> PathBuf {
    path.components()
        .fold(PathBuf::new(), |mut normalized, component| {
            match component {
                Component::CurDir => {},
                Component::ParentDir => {
                    if !normalized.pop() {
                        normalized.push(component);
                    }
                },
                named => normalized.push(named),
            }
            normalized
        })
}

fn direct_path_attribute(attribute: &Attribute) -> Option<String> {
    attribute
        .path()
        .is_ident("path")
        .then(|| path_from_meta(&attribute.meta))
        .flatten()
}

fn conditional_path_attributes(attribute: &Attribute) -> Vec<String> {
    if !attribute.path().is_ident("cfg_attr") {
        return Vec::new();
    }
    let Meta::List(list) = &attribute.meta else {
        return Vec::new();
    };
    let Ok(metas) = parse_meta_list(list) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for meta in metas.iter().skip(1) {
        collect_conditional_paths(meta, &mut paths);
    }
    paths
}

fn collect_conditional_paths(meta: &Meta, paths: &mut Vec<String>) {
    if meta.path().is_ident("path") {
        if let Some(path) = path_from_meta(meta) {
            paths.push(path);
        }
        return;
    }
    if !meta.path().is_ident("cfg_attr") {
        return;
    }
    let Meta::List(list) = meta else {
        return;
    };
    let Ok(metas) = parse_meta_list(list) else {
        return;
    };
    for nested in metas.iter().skip(1) {
        collect_conditional_paths(nested, paths);
    }
}

fn path_from_meta(meta: &Meta) -> Option<String> {
    let Meta::NameValue(name_value) = meta else {
        return None;
    };
    let Expr::Lit(expr_lit) = &name_value.value else {
        return None;
    };
    let Lit::Str(path) = &expr_lit.lit else {
        return None;
    };
    Some(path.value())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::FileModulePath;
    use super::ModuleDirectories;
    use super::ModuleMap;

    /// A `#[path]` written at the top level of a non-`mod.rs` file resolves
    /// against that file's directory, while a plain `mod name;` in the same file
    /// resolves against the module's own directory.
    #[test]
    fn top_level_path_attribute_resolves_against_the_file_directory() {
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/a/b.rs"));
        assert_eq!(directories.file, Path::new("/repo/src/a"));
        assert_eq!(directories.module, Path::new("/repo/src/a/b"));
    }

    #[test]
    fn a_mod_rs_file_resolves_both_bases_against_its_own_directory() {
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/a/mod.rs"));
        assert_eq!(directories.file, Path::new("/repo/src/a"));
        assert_eq!(directories.module, Path::new("/repo/src/a"));
    }

    #[test]
    fn an_inline_module_moves_both_bases_into_its_directory() {
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/a/b.rs"))
            .inside_inline_module("inner");
        assert_eq!(directories.file, Path::new("/repo/src/a/b/inner"));
        assert_eq!(directories.module, Path::new("/repo/src/a/b/inner"));
    }

    #[test]
    fn a_crate_root_resolves_both_bases_against_its_own_directory() {
        let directories = ModuleDirectories::for_crate_root(Path::new("/repo/src/bin/tool.rs"));
        assert_eq!(directories.file, Path::new("/repo/src/bin"));
        assert_eq!(directories.module, Path::new("/repo/src/bin"));
    }

    /// rustc resolves `mod helper;` in the binary root `src/bin/tool.rs`
    /// against `src/bin`, not against `src/bin/tool` as it would for a
    /// non-root file named `tool.rs`.
    #[test]
    fn a_binary_root_declares_modules_beside_it() {
        let crate_dir = tempdir().expect("create temp crate");
        let source_root = crate_dir.path().join("src");
        let binary_dir = source_root.join("bin");
        fs::create_dir_all(binary_dir.join("tool")).expect("create bin dirs");
        fs::write(
            binary_dir.join("tool.rs"),
            "mod helper;
fn main() {}
",
        )
        .expect("write binary root");
        fs::write(binary_dir.join("helper.rs"), "").expect("write sibling helper");
        fs::write(binary_dir.join("tool/helper.rs"), "").expect("write nested helper");
        let module_map = ModuleMap::for_crate_root(&binary_dir.join("tool.rs"));

        assert_eq!(
            module_map.file_module_path(&source_root, &binary_dir.join("helper.rs")),
            Some(FileModulePath::Known(vec!["helper".to_string()]))
        );
        assert_eq!(
            module_map.file_module_path(&source_root, &binary_dir.join("tool/helper.rs")),
            Some(FileModulePath::Known(vec![
                "bin".to_string(),
                "tool".to_string(),
                "helper".to_string()
            ]))
        );
    }

    /// A root outside `src`, such as an integration test, walks the same way.
    #[test]
    fn one_crate_root_resolves_only_its_own_modules() {
        let crate_dir = crate_with_detached_module();
        let tests_dir = crate_dir.path().join("tests");
        fs::create_dir_all(tests_dir.join("common")).expect("create tests dirs");
        fs::write(
            tests_dir.join("it.rs"),
            "mod common;
",
        )
        .expect("write test root");
        fs::write(
            tests_dir.join("common/mod.rs"),
            "mod fixtures;
",
        )
        .expect("write common");
        fs::write(tests_dir.join("common/fixtures.rs"), "").expect("write fixtures");
        let module_map = ModuleMap::for_crate_root(&tests_dir.join("it.rs"));

        assert_eq!(
            module_map.file_module_path(&tests_dir, &tests_dir.join("common/fixtures.rs")),
            Some(FileModulePath::Known(vec![
                "common".to_string(),
                "fixtures".to_string()
            ]))
        );
        let source_root = crate_dir.path().join("src");
        assert_eq!(
            module_map.file_module_path(&source_root, &source_root.join("stream/macos.rs")),
            Some(FileModulePath::Known(vec![
                "stream".to_string(),
                "macos".to_string()
            ]))
        );
    }

    /// The layout says `crate::stream::macos`; the `mod` declarations say
    /// `crate::platform::camera_stream`, and only the declarations decide what
    /// `super` names.
    #[test]
    fn a_path_attribute_moves_a_file_off_its_directory_module_path() {
        let crate_dir = crate_with_detached_module();
        let source_root = crate_dir.path().join("src");
        let module_map = ModuleMap::resolve(&source_root);

        assert_eq!(
            module_map.file_module_path(&source_root, &source_root.join("stream/macos.rs")),
            Some(FileModulePath::Known(vec![
                "platform".to_string(),
                "camera_stream".to_string()
            ]))
        );
        assert_eq!(
            module_map.file_module_path(&source_root, &source_root.join("stream/mod.rs")),
            Some(FileModulePath::Known(vec!["stream".to_string()]))
        );
    }

    #[test]
    fn a_file_no_mod_declaration_reaches_falls_back_to_its_directory() {
        let crate_dir = crate_with_detached_module();
        let source_root = crate_dir.path().join("src");
        fs::write(source_root.join("stream/orphan.rs"), "").expect("write orphan");
        let module_map = ModuleMap::resolve(&source_root);

        assert_eq!(
            module_map.file_module_path(&source_root, &source_root.join("stream/orphan.rs")),
            Some(FileModulePath::Known(vec![
                "stream".to_string(),
                "orphan".to_string()
            ]))
        );
    }

    #[test]
    fn a_file_two_parents_declare_has_no_single_module_path() {
        let crate_dir = crate_with_detached_module();
        let source_root = crate_dir.path().join("src");
        fs::write(
            source_root.join("platform/mod.rs"),
            "#[path = \"../stream/macos.rs\"]\nmod camera_stream;\n\
             #[path = \"../stream/macos.rs\"]\nmod camera_stream_again;\n",
        )
        .expect("write platform module");
        let module_map = ModuleMap::resolve(&source_root);

        assert_eq!(
            module_map.file_module_path(&source_root, &source_root.join("stream/macos.rs")),
            Some(FileModulePath::SeveralParents)
        );
    }

    /// A file compiles only under test when its own declaration or an
    /// ancestor's, file or inline, carries `#[cfg(test)]`.
    #[test]
    fn a_cfg_test_declaration_marks_its_file_and_the_files_below_it_test_only() {
        let crate_dir = tempdir().expect("create temp crate");
        let source_root = crate_dir.path().join("src");
        fs::create_dir_all(source_root.join("tests")).expect("create tests dir");
        fs::create_dir_all(source_root.join("gated/inner")).expect("create gated dir");
        fs::write(
            source_root.join("lib.rs"),
            "mod plain;\n#[cfg(test)]\nmod tests;\n\
             #[cfg(test)]\nmod gated {\n    mod inner;\n}\n",
        )
        .expect("write lib root");
        fs::write(source_root.join("plain.rs"), "").expect("write plain");
        fs::write(source_root.join("tests/mod.rs"), "mod helper;\n").expect("write tests");
        fs::write(source_root.join("tests/helper.rs"), "").expect("write helper");
        fs::write(source_root.join("gated/inner.rs"), "").expect("write inner");
        let module_map = ModuleMap::resolve(&source_root);

        assert!(!module_map.is_test_only(&source_root.join("lib.rs")));
        assert!(!module_map.is_test_only(&source_root.join("plain.rs")));
        assert!(module_map.is_test_only(&source_root.join("tests/mod.rs")));
        assert!(module_map.is_test_only(&source_root.join("tests/helper.rs")));
        assert!(module_map.is_test_only(&source_root.join("gated/inner.rs")));
    }

    /// `src/stream/macos.rs` sits under `stream` but is declared by `platform`,
    /// the layout cargo-mend used to read as `crate::stream::macos`.
    fn crate_with_detached_module() -> TempDir {
        let crate_dir = tempdir().expect("create temp crate");
        let source_root = crate_dir.path().join("src");
        fs::create_dir_all(source_root.join("platform")).expect("create platform dir");
        fs::create_dir_all(source_root.join("stream")).expect("create stream dir");
        fs::write(source_root.join("lib.rs"), "mod platform;\nmod stream;\n")
            .expect("write lib root");
        fs::write(
            source_root.join("platform/mod.rs"),
            "#[path = \"../stream/macos.rs\"]\nmod camera_stream;\n",
        )
        .expect("write platform module");
        fs::write(
            source_root.join("stream/mod.rs"),
            "pub(crate) struct CameraFrame;\n",
        )
        .expect("write stream module");
        fs::write(
            source_root.join("stream/macos.rs"),
            "use crate::stream::CameraFrame;\n",
        )
        .expect("write detached module");
        crate_dir
    }
}

use std::collections::hash_map::Entry;
use std::ffi::OsStr;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use syn::Expr;
use syn::Item;
use syn::ItemMod;
use syn::Lit;
use syn::Meta;
use syn::ext::IdentExt;

use super::ParsedSources;
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
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ModuleDirectories {
    file:   PathBuf,
    module: PathBuf,
}

enum ModulePathAttribute {
    Direct(String),
    Conditional(Vec<Self>),
    Ignored,
}

impl From<&Meta> for ModulePathAttribute {
    fn from(meta: &Meta) -> Self {
        match meta {
            Meta::NameValue(value) => {
                if value.path.is_ident("path") {
                    path_from_meta(meta).map_or(Self::Ignored, Self::Direct)
                } else {
                    Self::Ignored
                }
            },
            Meta::List(list) => {
                if list.path.is_ident("cfg_attr") {
                    parse_meta_list(list).map_or(Self::Ignored, |metas| {
                        Self::Conditional(metas.iter().skip(1).map(Self::from).collect())
                    })
                } else {
                    Self::Ignored
                }
            },
            Meta::Path(_) => Self::Ignored,
        }
    }
}

impl ModulePathAttribute {
    fn collect_paths(self, paths: &mut Vec<String>) {
        match self {
            Self::Direct(path) => paths.push(path),
            Self::Conditional(attributes) => {
                for attribute in attributes {
                    attribute.collect_paths(paths);
                }
            },
            Self::Ignored => {},
        }
    }
}

enum ModuleMountPaths {
    Default {
        conditional: Vec<String>,
    },
    Explicit {
        direct:      Vec<String>,
        conditional: Vec<String>,
    },
}

impl From<&ItemMod> for ModuleMountPaths {
    fn from(declaration: &ItemMod) -> Self {
        let mut direct = Vec::new();
        let mut conditional = Vec::new();
        for attribute in &declaration.attrs {
            match ModulePathAttribute::from(&attribute.meta) {
                ModulePathAttribute::Direct(path) => direct.push(path),
                ModulePathAttribute::Conditional(attributes) => {
                    for attribute in attributes {
                        attribute.collect_paths(&mut conditional);
                    }
                },
                ModulePathAttribute::Ignored => {},
            }
        }
        if direct.is_empty() {
            Self::Default { conditional }
        } else {
            Self::Explicit {
                direct,
                conditional,
            }
        }
    }
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

    /// The bases a child declaration inside an inline module resolves against.
    ///
    /// An inline module's own `#[path]` replaces its directory. Conditional
    /// paths retain every candidate because this walk does not evaluate cfg.
    pub(crate) fn inside_inline_module(&self, declaration: &ItemMod) -> Vec<Self> {
        let (mut directories, conditional) = match ModuleMountPaths::from(declaration) {
            ModuleMountPaths::Default { conditional } => {
                let module = self.module.join(declaration.ident.unraw().to_string());
                (
                    vec![Self {
                        file: module.clone(),
                        module,
                    }],
                    conditional,
                )
            },
            ModuleMountPaths::Explicit {
                direct,
                conditional,
            } => (
                direct.iter().map(|path| self.inline_path(path)).collect(),
                conditional,
            ),
        };
        for path in conditional {
            let directory = self.inline_path(&path);
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        }
        directories
    }

    fn inline_path(&self, path: &str) -> Self {
        let module = self.file.join(path);
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
        let (mut candidates, conditional) = match ModuleMountPaths::from(declaration) {
            ModuleMountPaths::Default { conditional } => (
                vec![
                    self.module.join(format!("{module_name}.rs")),
                    self.module.join(module_name).join("mod.rs"),
                ],
                conditional,
            ),
            ModuleMountPaths::Explicit {
                direct,
                conditional,
            } => (
                direct.iter().map(|path| self.file.join(path)).collect(),
                conditional,
            ),
        };
        for path in conditional {
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
    declarations: FxHashMap<PathBuf, DeclaredFile>,
}

impl ModuleMap {
    /// Resolve every file reachable from a crate root under `source_root`,
    /// reading each through `sources`. Callers ask
    /// [`ParsedSources::module_map`], which resolves each source root once.
    pub(super) fn resolve(source_root: &Path, sources: &ParsedSources) -> Self {
        Self::for_crate_roots(&crate_root_files(source_root), sources)
    }

    /// Merge each target's mount chains before deciding whether a file has a
    /// single module path suitable for relative rewrites.
    pub(super) fn for_crate_roots(crate_roots: &[PathBuf], sources: &ParsedSources) -> Self {
        let mut merged = Self {
            declarations: FxHashMap::default(),
        };
        for crate_root in crate_roots {
            merged.absorb(Self::for_crate_root(crate_root, sources));
        }
        merged
    }

    /// Resolve every file reachable from the one crate root `crate_root`,
    /// which may sit anywhere: `src/lib.rs`, `src/bin/tool.rs`, `tests/it.rs`.
    pub(crate) fn for_crate_root(crate_root: &Path, sources: &ParsedSources) -> Self {
        let mut walk = ModuleWalk {
            sources,
            declarations: FxHashMap::default(),
            visiting: FxHashSet::default(),
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
        }
    }

    fn absorb(&mut self, other: Self) {
        for (file, declared) in other.declarations {
            match self.declarations.entry(file) {
                Entry::Vacant(entry) => {
                    entry.insert(declared);
                },
                Entry::Occupied(mut entry) => entry.get_mut().merge(declared),
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
        self.declarations
            .iter()
            .map(|(file, declared)| (file.as_path(), declared.mounts.module_path()))
    }

    /// Whether every `mod` declaration chain reaching `file` carries
    /// `#[cfg(test)]` on the file's own declaration or an ancestor's, so the
    /// file compiles only under test. `false` for a file no chain declares.
    pub(crate) fn is_test_only(&self, file: &Path) -> bool {
        let file = lexically_normalized(file);
        self.declarations
            .get(&file)
            .is_some_and(|declared| matches!(declared.builds, DeclaredBuilds::TestOnly))
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
            .map(|declared| declared.mounts.module_path())
            .or_else(|| file_module_path(source_root, file).map(FileModulePath::Known))
    }
}

/// A declared file always has a first mount, so it cannot disappear from iteration or
/// fall back to its directory layout because its declaration entry was empty.
struct DeclaredMounts {
    first:      Vec<String>,
    additional: Vec<Vec<String>>,
}

impl DeclaredMounts {
    fn insert(&mut self, module_path: Vec<String>) {
        if module_path != self.first && !self.additional.contains(&module_path) {
            self.additional.push(module_path);
        }
    }

    fn module_path(&self) -> FileModulePath {
        if self.additional.is_empty() {
            FileModulePath::Known(self.first.clone())
        } else {
            FileModulePath::SeveralParents
        }
    }
}

/// Build reach belongs to a declared file, never to an independent membership set.
/// A file is test-only only when every mount chain includes `#[cfg(test)]`.
#[derive(Clone, Copy)]
enum DeclaredBuilds {
    AllBuilds,
    TestOnly,
}

impl DeclaredBuilds {
    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::TestOnly, Self::TestOnly) => Self::TestOnly,
            (Self::AllBuilds, Self::AllBuilds | Self::TestOnly)
            | (Self::TestOnly, Self::AllBuilds) => Self::AllBuilds,
        }
    }
}

struct DeclaredFile {
    mounts: DeclaredMounts,
    builds: DeclaredBuilds,
}

impl DeclaredFile {
    fn merge(&mut self, other: Self) {
        self.builds = self.builds.merge(other.builds);
        self.mounts.insert(other.mounts.first);
        for module_path in other.mounts.additional {
            self.mounts.insert(module_path);
        }
    }
}

/// A module the walk reached: its path, and whether a `#[cfg(test)]` on its
/// declaration or an ancestor's limits it to test builds.
struct Declared {
    module_path: Vec<String>,
    test_only:   bool,
}

struct ModuleWalk<'a> {
    sources:      &'a ParsedSources,
    declarations: FxHashMap<PathBuf, DeclaredFile>,
    /// The `(file, module path)` pairs already being walked, so a `#[path]`
    /// cycle terminates while a file declared at two paths still records both.
    visiting:     FxHashSet<(PathBuf, Vec<String>)>,
}

impl ModuleWalk<'_> {
    fn declare(&mut self, file: &Path, module: &Declared) {
        let file = lexically_normalized(file);
        let builds = if module.test_only {
            DeclaredBuilds::TestOnly
        } else {
            DeclaredBuilds::AllBuilds
        };
        match self.declarations.entry(file) {
            Entry::Vacant(entry) => {
                entry.insert(DeclaredFile {
                    mounts: DeclaredMounts {
                        first:      module.module_path.clone(),
                        additional: Vec::new(),
                    },
                    builds,
                });
            },
            Entry::Occupied(mut entry) => {
                let declared = entry.get_mut();
                declared.builds = declared.builds.merge(builds);
                if module.module_path != declared.mounts.first
                    && !declared.mounts.additional.contains(&module.module_path)
                {
                    declared.mounts.additional.push(module.module_path.clone());
                }
            },
        }
    }

    fn walk_file(&mut self, file: &Path, module: &Declared, directories: &ModuleDirectories) {
        let visit = (lexically_normalized(file), module.module_path.clone());
        if !self.visiting.insert(visit.clone()) {
            return;
        }
        if let Ok(source) = self.sources.source(file)
            && let Ok(syntax) = source.syntax()
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
                for directory in directories.inside_inline_module(declaration) {
                    self.walk_items(inline_items, &directory, &child);
                }
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
pub(super) fn lexically_normalized(path: &Path) -> PathBuf {
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
    use crate::rust_syntax::ParsedSources;

    #[test]
    fn declarations_keep_the_first_mount_including_the_empty_crate_root_path() {
        let sources = ParsedSources::default();
        let mut walk = super::ModuleWalk {
            sources:      &sources,
            declarations: rustc_hash::FxHashMap::default(),
            visiting:     rustc_hash::FxHashSet::default(),
        };
        let file = Path::new("/repo/src/shared.rs");
        let root = super::Declared {
            module_path: Vec::new(),
            test_only:   true,
        };
        walk.declare(file, &root);
        walk.declare(file, &root);
        let mut map = ModuleMap {
            declarations: walk.declarations,
        };
        assert!(map.is_test_only(file));
        assert!(!map.is_test_only(Path::new("/repo/src/undeclared.rs")));
        assert_eq!(
            map.declared_files().collect::<Vec<_>>(),
            vec![(file, FileModulePath::Known(Vec::new()))],
        );
        assert_eq!(
            map.file_module_path(Path::new("/repo/src"), file),
            Some(FileModulePath::Known(Vec::new())),
        );
        let declared = map.declarations.get_mut(file).expect("declared root");
        declared.mounts.insert(vec!["second".to_string()]);
        declared.mounts.insert(vec!["second".to_string()]);
        assert_eq!(declared.mounts.additional.len(), 1);
        assert_eq!(
            map.declared_files().collect::<Vec<_>>(),
            vec![(file, FileModulePath::SeveralParents)],
        );
    }

    #[test]
    fn merging_mount_chains_keeps_all_builds_when_any_target_is_ungated() {
        let crate_dir = tempdir().expect("create merged mount crate");
        let source_root = crate_dir.path();
        let gated = source_root.join("gated.rs");
        let plain = source_root.join("plain.rs");
        let shared = source_root.join("shared.rs");
        fs::write(&gated, "#[cfg(test)] #[path = \"shared.rs\"] mod shared;")
            .expect("write gated root");
        fs::write(&plain, "#[path = \"shared.rs\"] mod shared;").expect("write plain root");
        fs::write(&shared, "").expect("write shared module");
        let sources = ParsedSources::default();
        let gated_map = ModuleMap::for_crate_roots(&[gated.clone(), gated.clone()], &sources);
        assert!(gated_map.is_test_only(&shared));
        for roots in [
            [gated.clone(), plain.clone()],
            [plain.clone(), gated],
            [plain.clone(), plain],
        ] {
            let map = ModuleMap::for_crate_roots(&roots, &sources);
            assert!(!map.is_test_only(&shared));
            assert_eq!(
                map.file_module_path(source_root, &shared),
                Some(FileModulePath::Known(vec!["shared".to_string()])),
            );
            assert_eq!(map.declarations[&shared].mounts.additional.len(), 0);
        }
    }

    #[test]
    fn module_mounts_preserve_explicit_precedence_and_nested_conditional_order() {
        let declaration = syn::parse_str(concat!(
            "#[cfg_attr(unix, path = \"conditional.rs\", ",
            "cfg_attr(test, path = \"nested.rs\"))] ",
            "#[path = \"direct.rs\"] ",
            "#[cfg_attr(windows, path = \"direct.rs\", path = \"conditional.rs\")] ",
            "mod r#type;",
        ))
        .expect("parse mixed mounts");
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/owner.rs"));
        let expected = ["direct.rs", "conditional.rs", "nested.rs"]
            .map(|name| Path::new("/repo/src").join(name));
        assert_eq!(directories.declared_module_files(&declaration), expected);
        let inline = directories.inside_inline_module(&declaration);
        assert_eq!(
            inline
                .iter()
                .map(|directory| &directory.file)
                .collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
        assert!(
            inline
                .iter()
                .all(|directory| directory.file == directory.module)
        );
    }

    #[test]
    fn malformed_mounts_keep_defaults_and_valid_conditional_paths() {
        let declaration = syn::parse_str(concat!(
            "#[path] #[cfg_attr] ",
            "#[path = 7] #[cfg_attr(unix, path = 8, cfg_attr(test, path = \"test.rs\"))] ",
            "#[cfg_attr(unix, path =)] #[other(path = \"ignored.rs\")] mod r#type;",
        ))
        .expect("parse malformed mount payloads");
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/owner.rs"));
        assert_eq!(
            directories.declared_module_files(&declaration),
            vec![
                Path::new("/repo/src/owner/type.rs").to_path_buf(),
                Path::new("/repo/src/owner/type/mod.rs").to_path_buf(),
                Path::new("/repo/src/test.rs").to_path_buf(),
            ]
        );
        let inline = directories.inside_inline_module(&declaration);
        assert_eq!(
            inline
                .iter()
                .map(|directory| directory.file.as_path())
                .collect::<Vec<_>>(),
            vec![
                Path::new("/repo/src/owner/type"),
                Path::new("/repo/src/test.rs")
            ]
        );
    }

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
        let declaration = syn::parse_str("mod inner {}").expect("parse inline module");
        let directories = ModuleDirectories::for_file(Path::new("/repo/src/a/b.rs"))
            .inside_inline_module(&declaration)
            .remove(0);
        assert_eq!(directories.file, Path::new("/repo/src/a/b/inner"));
        assert_eq!(directories.module, Path::new("/repo/src/a/b/inner"));
    }

    #[test]
    fn an_inline_path_attribute_resolves_child_files_at_its_declared_directory() {
        let crate_dir = tempdir().expect("create inline mount crate");
        let source_root = crate_dir.path().join("src");
        fs::create_dir_all(source_root.join("parent")).expect("create parent directory");
        fs::write(source_root.join("lib.rs"), "mod parent;\n").expect("write crate root");
        fs::write(source_root.join("parent/mod.rs"), "mod scan;\n").expect("write parent");
        fs::write(
            source_root.join("parent/scan.rs"),
            "#[cfg(test)]\n#[path = \".\"]\nmod tests { #[path = \"x.rs\"] mod x; }\n",
        )
        .expect("write inline test mount");
        fs::write(source_root.join("parent/x.rs"), "").expect("write test child");
        let map = ModuleMap::resolve(&source_root, &ParsedSources::default());
        assert_eq!(
            map.file_module_path(&source_root, &source_root.join("parent/x.rs")),
            Some(FileModulePath::Known(vec![
                "parent".to_string(),
                "scan".to_string(),
                "tests".to_string(),
                "x".to_string(),
            ]))
        );
        assert!(map.is_test_only(&source_root.join("parent/x.rs")));
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
        let module_map =
            ModuleMap::for_crate_root(&binary_dir.join("tool.rs"), &ParsedSources::default());

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
        let module_map =
            ModuleMap::for_crate_root(&tests_dir.join("it.rs"), &ParsedSources::default());

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
        let module_map = ModuleMap::resolve(&source_root, &ParsedSources::default());

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
        let module_map = ModuleMap::resolve(&source_root, &ParsedSources::default());

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
        let module_map = ModuleMap::resolve(&source_root, &ParsedSources::default());

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
        let module_map = ModuleMap::resolve(&source_root, &ParsedSources::default());

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

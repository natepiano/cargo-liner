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
}

impl ModuleMap {
    /// Resolve every file reachable from a crate root under `source_root`.
    pub(crate) fn resolve(source_root: &Path) -> Self {
        let mut walk = ModuleWalk {
            declarations: FxHashMap::default(),
            visiting:     FxHashSet::default(),
        };
        for crate_root in crate_root_files(source_root) {
            walk.declare(&crate_root, Vec::new());
            walk.walk_file(&crate_root, &[]);
        }
        Self {
            declarations: walk.declarations,
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
        match self
            .declarations
            .get(&lexically_normalized(file))
            .map(Vec::as_slice)
        {
            Some([module_path]) => Some(FileModulePath::Known(module_path.clone())),
            Some([_, _, ..]) => Some(FileModulePath::SeveralParents),
            Some([]) | None => file_module_path(source_root, file).map(FileModulePath::Known),
        }
    }
}

struct ModuleWalk {
    declarations: FxHashMap<PathBuf, Vec<Vec<String>>>,
    /// The `(file, module path)` pairs already being walked, so a `#[path]`
    /// cycle terminates while a file declared at two paths still records both.
    visiting:     FxHashSet<(PathBuf, Vec<String>)>,
}

impl ModuleWalk {
    fn declare(&mut self, file: &Path, module_path: Vec<String>) {
        let declared = self
            .declarations
            .entry(lexically_normalized(file))
            .or_default();
        if !declared.contains(&module_path) {
            declared.push(module_path);
        }
    }

    fn walk_file(&mut self, file: &Path, module_path: &[String]) {
        let visit = (lexically_normalized(file), module_path.to_vec());
        if !self.visiting.insert(visit.clone()) {
            return;
        }
        if let Ok(text) = fs::read_to_string(file)
            && let Ok(syntax) = parse_file(&text)
        {
            self.walk_items(
                &syntax.items,
                &ModuleDirectories::for_file(file),
                module_path,
            );
        }
        self.visiting.remove(&visit);
    }

    fn walk_items(
        &mut self,
        items: &[Item],
        directories: &ModuleDirectories,
        module_path: &[String],
    ) {
        for item in items {
            let Item::Mod(declaration) = item else {
                continue;
            };
            let module_name = declaration.ident.unraw().to_string();
            let mut child_path = module_path.to_vec();
            child_path.push(module_name.clone());

            if let Some((_, inline_items)) = &declaration.content {
                self.walk_items(
                    inline_items,
                    &directories.inside_inline_module(&module_name),
                    &child_path,
                );
                continue;
            }

            for module_file in directories.declared_module_files(declaration) {
                if !module_file.is_file() {
                    continue;
                }
                self.declare(&module_file, child_path.clone());
                self.walk_file(&module_file, &child_path);
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

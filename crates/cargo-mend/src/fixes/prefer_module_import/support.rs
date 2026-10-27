use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;

use crate::fixes::imports::ImportSource;
use crate::rust_syntax;
use crate::rust_syntax::ParsedSources;
use crate::rust_syntax::PathAnchor;

/// A crate's source root, and the sources its module files are read through.
#[derive(Clone, Copy)]
pub(super) struct CrateSources<'a> {
    pub(super) source_root: &'a Path,
    pub(super) sources:     &'a ParsedSources,
}

/// A flattened leaf import always names a leaf, including when it has no prefix.
/// Re-export lookup therefore never has to recover from an empty import path.
pub(super) struct FlattenedImport {
    pub(super) prefix: Vec<String>,
    pub(super) leaf:   String,
    pub(super) rename: Option<String>,
}

impl FlattenedImport {
    fn anchor(&self) -> PathAnchor {
        PathAnchor::from(self.prefix.first().unwrap_or(&self.leaf).as_str())
    }

    pub(super) fn absolute_path(&self, current_module_path: &[String]) -> Option<Vec<String>> {
        let segments = self.prefix.iter().chain(std::iter::once(&self.leaf));
        match self.anchor() {
            PathAnchor::Crate => Some(segments.skip(1).cloned().collect()),
            PathAnchor::Super => {
                // The required leaf can itself be the last ancestor navigation segment.
                let super_count = segments
                    .clone()
                    .take_while(|segment| *segment == "super")
                    .count();
                let retained = current_module_path.len().checked_sub(super_count)?;
                let mut absolute = current_module_path[..retained].to_vec();
                absolute.extend(segments.skip(super_count).cloned());
                Some(absolute)
            },
            PathAnchor::SelfMod | PathAnchor::SelfType | PathAnchor::Name => None,
        }
    }
}

/// A source-bound position validated before it can participate in an edit.
/// Missing lines and columns must be rejected, never repaired to byte zero.
pub(super) struct SourcePosition<'a> {
    byte:        usize,
    source_line: &'a str,
}

impl<'a> SourcePosition<'a> {
    pub(super) const fn byte(&self) -> usize { self.byte }

    pub(super) const fn source_line(&self) -> &'a str { self.source_line }
}

/// An ordered pair of positions checked together against one source owner.
pub(super) struct SourceSpan<'a> {
    start: SourcePosition<'a>,
    end:   usize,
}

impl<'a> SourceSpan<'a> {
    pub(super) const fn start(&self) -> &SourcePosition<'a> { &self.start }

    pub(super) const fn end(&self) -> usize { self.end }

    pub(super) fn new(
        source: &ImportSource<'a>,
        start: LineColumn,
        end: LineColumn,
    ) -> Result<Self> {
        let location = source
            .locate(start, end)
            .with_context(|| format!("invalid import source span {start:?}..{end:?}"))?;
        Ok(Self {
            start: SourcePosition {
                byte:        location.start(),
                source_line: location.source_line(),
            },
            end:   location.end(),
        })
    }
}

pub(super) fn resolve_to_absolute(
    segments: &[String],
    current_module_path: &[String],
) -> Option<Vec<String>> {
    match PathAnchor::first(segments)? {
        PathAnchor::Crate => Some(segments[1..].to_vec()),
        PathAnchor::Super => {
            let super_count = rust_syntax::leading_super_count(segments);
            if super_count > current_module_path.len() {
                return None;
            }
            let mut absolute =
                current_module_path[..current_module_path.len() - super_count].to_vec();
            absolute.extend(segments[super_count..].iter().cloned());
            Some(absolute)
        },
        PathAnchor::SelfMod | PathAnchor::SelfType | PathAnchor::Name => None,
    }
}

pub(super) fn leaf_is_module(
    crate_sources: CrateSources<'_>,
    absolute_segments: &[String],
) -> bool {
    let mut visited = BTreeSet::new();
    resolves_to_module(crate_sources, absolute_segments, &mut visited)
}

enum ModuleSegmentSource {
    File(PathBuf),
    DirectoryModule(PathBuf),
    InlineOrReexport,
}

impl ModuleSegmentSource {
    fn for_segment(dir: &Path, segment: &str) -> Self {
        let file = dir.join(format!("{segment}.rs"));
        let directory_module = dir.join(segment).join("mod.rs");
        if file.is_file() {
            Self::File(file)
        } else if directory_module.is_file() {
            Self::DirectoryModule(directory_module)
        } else {
            Self::InlineOrReexport
        }
    }
}

/// Walks `absolute_segments` from the crate root through file-backed modules,
/// then through inline `mod` blocks and `use` re-exports inside the last
/// file-backed module. `visited` holds the paths already under resolution, so a
/// re-export cycle ends instead of recursing forever.
fn resolves_to_module(
    crate_sources: CrateSources<'_>,
    absolute_segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> bool {
    if absolute_segments.is_empty() || !visited.insert(absolute_segments.to_vec()) {
        return false;
    }

    let mut dir = crate_sources.source_root.to_path_buf();
    let mut module_file = crate_root_file(crate_sources.source_root);
    let mut module_path: Vec<String> = Vec::new();
    for (index, segment) in absolute_segments.iter().enumerate() {
        match ModuleSegmentSource::for_segment(&dir, segment) {
            ModuleSegmentSource::File(file) | ModuleSegmentSource::DirectoryModule(file) => {
                module_file = Some(file);
            },
            ModuleSegmentSource::InlineOrReexport => {
                // Without a backing file, resolve the remainder inside the last file-backed module.
                let remainder = &absolute_segments[index..];
                return module_file.is_some_and(|module_file| {
                    declares_module(
                        crate_sources,
                        &module_file,
                        &module_path,
                        remainder,
                        visited,
                    )
                });
            },
        }
        module_path.push(segment.clone());
        dir.push(segment);
    }
    true
}

/// True when the module defined by `file` — whose absolute path is
/// `module_path` — makes `segments` reachable, either as inline `mod` blocks or
/// through a `use` re-export naming the first segment.
fn declares_module(
    crate_sources: CrateSources<'_>,
    file: &Path,
    module_path: &[String],
    segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> bool {
    let Some(first) = segments.first() else {
        return false;
    };
    let Ok(source) = crate_sources.sources.source(file) else {
        return false;
    };
    if !source.text().contains(first.as_str()) {
        return false;
    }
    let Ok(syntax) = source.syntax() else {
        return false;
    };
    declares_inline_mod_path(syntax, segments)
        || reexports_module(crate_sources, syntax, module_path, segments, visited)
}

fn crate_root_file(source_root: &Path) -> Option<PathBuf> {
    ["lib.rs", "main.rs"]
        .iter()
        .map(|name| source_root.join(name))
        .find(|path| path.is_file())
}

/// True when `syntax` declares `segments` as a chain of nested `mod` items,
/// e.g. `["parameter_fields"]` matches an inline `pub mod parameter_fields
/// { ... }`. Intermediate segments must be inline blocks (`mod x { ... }`); the
/// final segment may also be a boundary declaration (`mod x;`).
fn declares_inline_mod_path(syntax: &File, segments: &[String]) -> bool {
    let Some((leaf, parents)) = segments.split_last() else {
        return false;
    };

    let mut items = &syntax.items;
    for segment in parents {
        match find_mod_item(items, segment).and_then(|item_mod| item_mod.content.as_ref()) {
            Some((_, content)) => items = content,
            None => return false,
        }
    }
    find_mod_item(items, leaf).is_some()
}

/// True when `syntax` re-exports `segments[0]` with a `use` whose target
/// resolves to a module, e.g. `pub(crate) use self::plane::proof_fixture;`
/// naming the inline `mod proof_fixture` a file further down. A private `use`
/// binds nothing outside its own module, so only a `pub`-flavoured one counts.
fn reexports_module(
    crate_sources: CrateSources<'_>,
    syntax: &File,
    module_path: &[String],
    segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> bool {
    let Some((name, tail)) = segments.split_first() else {
        return false;
    };

    syntax.items.iter().any(|item| match item {
        Item::Use(item_use) if !matches!(item_use.vis, Visibility::Inherited) => {
            reexport_target(item_use, module_path, name).is_some_and(|target| {
                let mut target = target;
                target.extend(tail.iter().cloned());
                resolves_to_module(crate_sources, &target, visited)
            })
        },
        _ => false,
    })
}

/// The absolute crate path `item_use` binds `name` to, when it binds `name` at
/// all. `module_path` is the module holding the `use`, which anchors `self::`
/// and `super::` targets; a bare-name target is another crate and resolves to
/// `None`.
fn reexport_target(item_use: &ItemUse, module_path: &[String], name: &str) -> Option<Vec<String>> {
    let flat = flatten_use_tree(&item_use.tree)?;
    let bound = flat.rename.as_ref().unwrap_or(&flat.leaf);
    if bound != name {
        return None;
    }
    match flat.anchor() {
        PathAnchor::SelfMod => {
            let mut absolute = module_path.to_vec();
            absolute.extend(flat.prefix.iter().skip(1).cloned());
            if !flat.prefix.is_empty() {
                absolute.push(flat.leaf.clone());
            }
            Some(absolute)
        },
        PathAnchor::Crate | PathAnchor::Super | PathAnchor::SelfType | PathAnchor::Name => {
            flat.absolute_path(module_path)
        },
    }
}

fn find_mod_item<'a>(items: &'a [Item], name: &str) -> Option<&'a ItemMod> {
    items.iter().find_map(|item| match item {
        Item::Mod(item_mod) if item_mod.ident == name => Some(item_mod),
        _ => None,
    })
}

pub(super) fn shorten_module_path(
    current_module_path: &[String],
    module_segments: &[String],
) -> Vec<String> {
    match PathAnchor::first(module_segments) {
        Some(PathAnchor::Super) | None => return module_segments.to_vec(),
        Some(PathAnchor::Crate) => {},
        Some(PathAnchor::SelfMod | PathAnchor::SelfType | PathAnchor::Name) => {
            return module_segments.to_vec();
        },
    }

    let target = &module_segments[1..];
    if target.is_empty() {
        return module_segments.to_vec();
    }

    let common = common_prefix_len(current_module_path, target);
    if common == 0 {
        return module_segments.to_vec();
    }

    let up_count = current_module_path.len().saturating_sub(common);
    if up_count > 1 {
        return module_segments.to_vec();
    }

    let mut relative = Vec::new();
    if up_count == 1 {
        relative.push("super".to_string());
    }
    relative.extend(target[common..].iter().cloned());

    if relative.is_empty() || relative == module_segments[1..] {
        return module_segments.to_vec();
    }

    relative
}

pub(super) fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

pub(super) fn flatten_use_tree(tree: &UseTree) -> Option<FlattenedImport> {
    let mut segments = Vec::new();
    let mut cursor = tree;
    loop {
        match cursor {
            UseTree::Path(path) => {
                segments.push(path.ident.to_string());
                cursor = &path.tree;
            },
            UseTree::Name(name) => {
                break Some(FlattenedImport {
                    prefix: segments,
                    leaf:   name.ident.to_string(),
                    rename: None,
                });
            },
            UseTree::Rename(rename_tree) => {
                break Some(FlattenedImport {
                    prefix: segments,
                    leaf:   rename_tree.ident.to_string(),
                    rename: Some(rename_tree.rename.to_string()),
                });
            },
            UseTree::Group(_) | UseTree::Glob(_) => break None,
        }
    }
}

pub(super) fn is_snake_case_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    if name.chars().all(|character| {
        character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit()
    }) {
        return false;
    }
    name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    })
}

pub(super) fn is_snake_case_module_name(name: &str) -> bool { is_snake_case_function_name(name) }

pub(super) fn offset<'a>(
    source: &ImportSource<'a>,
    position: LineColumn,
) -> Result<SourcePosition<'a>> {
    Ok(SourceSpan::new(source, position, position)?.start)
}

#[cfg(test)]
mod tests {
    use proc_macro2::LineColumn;
    use syn::ItemUse;

    use super::is_snake_case_function_name;
    use super::shorten_module_path;

    #[test]
    fn module_resolution_prefers_files_then_directory_modules_then_inline() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join("child"))?;
        std::fs::write(
            dir.path().join("lib.rs"),
            "mod child; mod inline { pub mod leaf {} }",
        )?;
        std::fs::write(dir.path().join("child.rs"), "")?;
        std::fs::write(dir.path().join("child/mod.rs"), "pub mod leaf {}")?;
        let sources = crate::rust_syntax::ParsedSources::default();
        let crate_sources = super::CrateSources {
            source_root: dir.path(),
            sources:     &sources,
        };
        let child = ["child".to_string(), "leaf".to_string()];
        assert!(!super::leaf_is_module(crate_sources, &child));
        std::fs::remove_file(dir.path().join("child.rs"))?;
        assert!(super::leaf_is_module(crate_sources, &child));
        assert!(super::leaf_is_module(
            crate_sources,
            &["inline".to_string(), "leaf".to_string()]
        ));
        Ok(())
    }

    #[test]
    fn shorten_super_returns_for_sibling() {
        let current = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let module = vec![
            "crate".to_string(),
            "a".to_string(),
            "b".to_string(),
            "sib".to_string(),
        ];
        assert_eq!(shorten_module_path(&current, &module), vec!["super", "sib"]);
    }

    #[test]
    fn shorten_to_bare_super_when_target_is_parent() {
        // current_module_path = a::b::c (file is a/b/c.rs)
        // target module = a::b (the file's own parent)
        // shortening collapses to bare ["super"] — the caller treats this as the
        // parent-module case and rewrites calls to `super::fn(...)` with no `use`.
        let current = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let module = vec!["crate".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(shorten_module_path(&current, &module), vec!["super"]);
    }

    #[test]
    fn shorten_keeps_absolute_when_no_common_prefix() {
        let current = vec!["a".to_string(), "b".to_string()];
        let module = vec!["crate".to_string(), "x".to_string(), "y".to_string()];
        assert_eq!(
            shorten_module_path(&current, &module),
            vec!["crate", "x", "y"]
        );
    }

    #[test]
    fn snake_case_detects_functions() {
        assert!(is_snake_case_function_name("do_thing"));
        assert!(is_snake_case_function_name("func_a"));
        assert!(is_snake_case_function_name("process_data"));
        assert!(is_snake_case_function_name("a"));
    }

    #[test]
    fn snake_case_rejects_types() {
        assert!(!is_snake_case_function_name("MyType"));
        assert!(!is_snake_case_function_name("Thing"));
        assert!(!is_snake_case_function_name("PublicContainer"));
    }

    #[test]
    fn snake_case_rejects_constants() {
        assert!(!is_snake_case_function_name("MAX_SIZE"));
        assert!(!is_snake_case_function_name("DEFAULT_PORT"));
    }

    #[test]
    fn snake_case_rejects_empty() {
        assert!(!is_snake_case_function_name(""));
    }
    #[test]
    fn checked_positions_reject_missing_coordinates_and_preserve_byte_boundaries()
    -> anyhow::Result<()> {
        let text = "// é\nα::run();\n";
        let source = crate::fixes::imports::ImportSource::new(text);
        let at = |line, column| LineColumn { line, column };
        for position in [at(0, 0), at(4, 0), at(1, 5), at(2, 10)] {
            assert!(super::offset(&source, position).is_err());
        }
        let position = super::offset(&source, at(2, 3))?;
        assert_eq!(position.byte(), "// é\nα::".len());
        assert_eq!(position.source_line(), "α::run();");
        let final_line = super::offset(&source, at(3, 0))?;
        assert_eq!(final_line.byte(), text.len());
        assert_eq!(final_line.source_line(), "");
        assert!(super::SourceSpan::new(&source, at(2, 4), at(2, 3)).is_err());
        Ok(())
    }

    #[test]
    fn flattened_path_anchors_match_absolute_and_reexport_resolution() -> anyhow::Result<()> {
        let current = ["a", "b", "c"].map(str::to_string);
        for (path, absolute, reexport) in [
            ("super::super", Some(vec!["a"]), Some(vec!["a"])),
            ("super::super::super", Some(vec![]), Some(vec![])),
            ("super::super::super::super", None, None),
            (
                "super::named_leaf",
                Some(vec!["a", "b", "named_leaf"]),
                Some(vec!["a", "b", "named_leaf"]),
            ),
            ("super", Some(vec!["a", "b"]), Some(vec!["a", "b"])),
            ("crate", Some(vec![]), Some(vec![])),
            (
                "crate::named_leaf",
                Some(vec!["named_leaf"]),
                Some(vec!["named_leaf"]),
            ),
            ("self", None, Some(vec!["a", "b", "c"])),
            (
                "self::named_leaf",
                None,
                Some(vec!["a", "b", "c", "named_leaf"]),
            ),
            ("external", None, None),
            ("external::named_leaf", None, None),
        ] {
            let item: ItemUse = syn::parse_str(&format!("pub use {path} as ancestor;"))?;
            let flat = super::flatten_use_tree(&item.tree)
                .ok_or_else(|| anyhow::anyhow!("expected a leaf import"))?;
            let owned = |segments: Vec<&str>| segments.into_iter().map(str::to_string).collect();
            assert_eq!(flat.absolute_path(&current), absolute.map(owned), "{path}");
            assert_eq!(
                super::reexport_target(&item, &current, "ancestor"),
                reexport.map(owned),
                "{path}",
            );
            assert_eq!(super::reexport_target(&item, &current, "unbound"), None);
        }
        Ok(())
    }

    #[test]
    fn flattened_imports_always_carry_the_leaf_separately() -> anyhow::Result<()> {
        for (text, prefix, leaf, rename) in [
            ("use run;", Vec::new(), "run", None),
            ("use crate::util::run;", vec!["crate", "util"], "run", None),
            (
                "use self::run as execute;",
                vec!["self"],
                "run",
                Some("execute"),
            ),
        ] {
            let item: ItemUse = syn::parse_str(text)?;
            let flat = super::flatten_use_tree(&item.tree)
                .ok_or_else(|| anyhow::anyhow!("expected a leaf import"))?;
            assert_eq!(flat.prefix, prefix);
            assert_eq!(flat.leaf, leaf);
            assert_eq!(flat.rename.as_deref(), rename);
        }
        let item: ItemUse = syn::parse_str("pub use self::run as execute;")?;
        assert_eq!(
            super::reexport_target(&item, &["util".to_string()], "execute"),
            Some(vec!["util".to_string(), "run".to_string()]),
        );
        for text in ["use crate::util::*;", "use crate::util::{run, stop};"] {
            let item: ItemUse = syn::parse_str(text)?;
            assert!(super::flatten_use_tree(&item.tree).is_none());
        }
        Ok(())
    }
}

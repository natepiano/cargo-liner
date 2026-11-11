use std::collections::BTreeSet;
use std::path::Path;

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

/// `module_resolution` returns `ModuleResolution::Module` for a path resolved through files,
/// inline blocks or re-exports, and `ModuleResolution::NotModule` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModuleResolution {
    Module,
    NotModule,
}

/// Classifies `absolute_segments` as `ModuleResolution::Module` or
/// `ModuleResolution::NotModule` through `resolves_to_module`, with a fresh cycle check.
pub(super) fn module_resolution(
    crate_sources: CrateSources<'_>,
    absolute_segments: &[String],
) -> ModuleResolution {
    let mut visited = BTreeSet::new();
    resolves_to_module(crate_sources, absolute_segments, &mut visited)
}

/// Resolve declared mounts before checking inline modules and re-exports.
/// `visited` prevents cycles through re-exports.
fn resolves_to_module(
    crate_sources: CrateSources<'_>,
    absolute_segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> ModuleResolution {
    if absolute_segments.is_empty() || !visited.insert(absolute_segments.to_vec()) {
        return ModuleResolution::NotModule;
    }

    if crate_sources
        .sources
        .module_map(crate_sources.source_root)
        .module_ancestors(absolute_segments)
        .any(|(file, module_path)| {
            let remainder = &absolute_segments[module_path.len()..];
            remainder.is_empty()
                || declares_module(crate_sources, file, module_path, remainder, visited)
                    == ModuleResolution::Module
        })
    {
        ModuleResolution::Module
    } else {
        ModuleResolution::NotModule
    }
}

/// `declares_module` returns `ModuleResolution::Module` when `file`, at absolute `module_path`,
/// makes `segments` reachable through inline `mod` blocks or a `use` re-export naming the first
/// segment, and `ModuleResolution::NotModule` otherwise.
fn declares_module(
    crate_sources: CrateSources<'_>,
    file: &Path,
    module_path: &[String],
    segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> ModuleResolution {
    let Some(first) = segments.first() else {
        return ModuleResolution::NotModule;
    };
    let Ok(parsed_source) = crate_sources.sources.source(file) else {
        return ModuleResolution::NotModule;
    };
    if !parsed_source.text().contains(first.as_str()) {
        return ModuleResolution::NotModule;
    }
    let Ok(parsed_file) = parsed_source.syntax() else {
        return ModuleResolution::NotModule;
    };
    match declares_inline_mod_path(parsed_file, segments) {
        ModuleResolution::Module => ModuleResolution::Module,
        ModuleResolution::NotModule => {
            reexports_module(crate_sources, parsed_file, module_path, segments, visited)
        },
    }
}

/// `declares_inline_mod_path` classifies nested module declarations with `ModuleResolution`.
/// e.g. `["parameter_fields"]` matches an inline `pub mod parameter_fields
/// { ... }`. Intermediate segments must be inline blocks (`mod x { ... }`); the
/// final segment may also be a boundary declaration (`mod x;`).
fn declares_inline_mod_path(syntax: &File, segments: &[String]) -> ModuleResolution {
    let Some((leaf, parents)) = segments.split_last() else {
        return ModuleResolution::NotModule;
    };

    let mut items = &syntax.items;
    for segment in parents {
        match find_mod_item(items, segment).and_then(|item_mod| item_mod.content.as_ref()) {
            Some((_, content)) => items = content,
            None => return ModuleResolution::NotModule,
        }
    }
    find_mod_item(items, leaf).map_or(ModuleResolution::NotModule, |_| ModuleResolution::Module)
}

/// `reexports_module` returns `ModuleResolution::Module` when `syntax` re-exports `segments[0]`
/// with a `use` whose target resolves to a module, e.g. `pub(crate) use
/// self::plane::proof_fixture;` naming the inline `mod proof_fixture` a file further down. A
/// private `use` binds nothing outside its own module, so only a `pub`-flavoured one counts.
/// Unresolved targets return `ModuleResolution::NotModule`.
fn reexports_module(
    crate_sources: CrateSources<'_>,
    syntax: &File,
    module_path: &[String],
    segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> ModuleResolution {
    let Some((name, tail)) = segments.split_first() else {
        return ModuleResolution::NotModule;
    };

    if syntax.items.iter().any(|item| match item {
        Item::Use(item_use) if !matches!(item_use.vis, Visibility::Inherited) => {
            reexport_target(item_use, module_path, name).is_some_and(|target| {
                let mut target = target;
                target.extend(tail.iter().cloned());
                resolves_to_module(crate_sources, &target, visited) == ModuleResolution::Module
            })
        },
        _ => false,
    }) {
        ModuleResolution::Module
    } else {
        ModuleResolution::NotModule
    }
}

/// The absolute crate path `item_use` binds `name` to, when it binds `name` at
/// all. `module_path` is the module holding the `use`, which anchors `self::`
/// and `super::` targets; a bare-name target is another crate and resolves to
/// `None`.
fn reexport_target(item_use: &ItemUse, module_path: &[String], name: &str) -> Option<Vec<String>> {
    let flattened_import = flatten_use_tree(&item_use.tree)?;
    let bound = flattened_import
        .rename
        .as_ref()
        .unwrap_or(&flattened_import.leaf);
    if bound != name {
        return None;
    }
    match flattened_import.anchor() {
        PathAnchor::SelfMod => {
            let mut absolute = module_path.to_vec();
            absolute.extend(flattened_import.prefix.iter().skip(1).cloned());
            if !flattened_import.prefix.is_empty() {
                absolute.push(flattened_import.leaf.clone());
            }
            Some(absolute)
        },
        PathAnchor::Crate | PathAnchor::Super | PathAnchor::SelfType | PathAnchor::Name => {
            flattened_import.absolute_path(module_path)
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

pub(super) fn flatten_use_tree(use_tree: &UseTree) -> Option<FlattenedImport> {
    let mut segments = Vec::new();
    let mut use_tree = use_tree;
    loop {
        match use_tree {
            UseTree::Path(path) => {
                segments.push(path.ident.to_string());
                use_tree = &path.tree;
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

/// `import_name_case` uses `ImportNameCase::SnakeCase` and `ImportNameCase::NotSnakeCase`
/// for the shared ASCII spelling rule for function and module import names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportNameCase {
    SnakeCase,
    NotSnakeCase,
}

/// Returns `ImportNameCase::SnakeCase` when `name` passes the shared ASCII spelling rule,
/// and `ImportNameCase::NotSnakeCase` otherwise.
pub(super) fn import_name_case(name: &str) -> ImportNameCase {
    let Some(first) = name.chars().next() else {
        return ImportNameCase::NotSnakeCase;
    };
    if !first.is_ascii_lowercase() && first != '_' {
        return ImportNameCase::NotSnakeCase;
    }
    if name.chars().all(|character| {
        character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit()
    }) {
        return ImportNameCase::NotSnakeCase;
    }
    if name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    }) {
        ImportNameCase::SnakeCase
    } else {
        ImportNameCase::NotSnakeCase
    }
}

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

    use super::ImportNameCase;
    use super::import_name_case;
    use super::shorten_module_path;

    #[test]
    fn path_mounted_import_resolution_uses_declared_module_paths() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        for (file, text) in [
            ("src/lib.rs", "mod propagation;"),
            (
                "src/propagation.rs",
                "#[path = \"../tests/roles/mod.rs\"] mod phase68_tests;",
            ),
            ("tests/roles/mod.rs", "pub(crate) mod support;"),
            (
                "tests/roles/support.rs",
                "pub(crate) fn value() -> usize { 7 }",
            ),
        ] {
            let file = dir.path().join(file);
            std::fs::create_dir_all(
                file.parent()
                    .ok_or_else(|| anyhow::anyhow!("source directory"))?,
            )?;
            std::fs::write(file, text)?;
        }
        let sources = crate::rust_syntax::ParsedSources::default();
        let source_root = dir.path().join("src");
        let map = sources.module_map(&source_root);
        let mounted = vec!["propagation".to_string(), "phase68_tests".to_string()];
        assert_eq!(
            map.file_module_path(&source_root, &dir.path().join("tests/roles/mod.rs")),
            Some(crate::rust_syntax::FileModulePath::Known(mounted.clone())),
        );
        let mut child = mounted.clone();
        child.push("support".to_string());
        assert_eq!(
            map.file_module_path(&source_root, &dir.path().join("tests/roles/support.rs")),
            Some(crate::rust_syntax::FileModulePath::Known(child.clone())),
        );
        let crate_sources = super::CrateSources {
            source_root: &source_root,
            sources:     &sources,
        };
        assert_eq!(
            super::module_resolution(crate_sources, &mounted),
            super::ModuleResolution::Module
        );
        assert_eq!(
            super::module_resolution(crate_sources, &child),
            super::ModuleResolution::Module
        );
        child.push("value".to_string());
        assert_eq!(
            super::module_resolution(crate_sources, &child),
            super::ModuleResolution::NotModule
        );
        Ok(())
    }

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
        assert_eq!(
            super::module_resolution(crate_sources, &child),
            super::ModuleResolution::NotModule
        );
        std::fs::remove_file(dir.path().join("child.rs"))?;
        let sources = crate::rust_syntax::ParsedSources::default();
        let crate_sources = super::CrateSources {
            source_root: dir.path(),
            sources:     &sources,
        };
        assert_eq!(
            super::module_resolution(crate_sources, &child),
            super::ModuleResolution::Module
        );
        assert_eq!(
            super::module_resolution(crate_sources, &["inline".to_string(), "leaf".to_string()]),
            super::ModuleResolution::Module
        );
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
        assert_eq!(import_name_case("do_thing"), ImportNameCase::SnakeCase);
        assert_eq!(import_name_case("func_a"), ImportNameCase::SnakeCase);
        assert_eq!(import_name_case("process_data"), ImportNameCase::SnakeCase);
        assert_eq!(import_name_case("a"), ImportNameCase::SnakeCase);
    }

    #[test]
    fn snake_case_rejects_types() {
        assert_eq!(import_name_case("MyType"), ImportNameCase::NotSnakeCase);
        assert_eq!(import_name_case("Thing"), ImportNameCase::NotSnakeCase);
        assert_eq!(
            import_name_case("PublicContainer"),
            ImportNameCase::NotSnakeCase
        );
    }

    #[test]
    fn snake_case_rejects_constants() {
        assert_eq!(import_name_case("MAX_SIZE"), ImportNameCase::NotSnakeCase);
        assert_eq!(
            import_name_case("DEFAULT_PORT"),
            ImportNameCase::NotSnakeCase
        );
    }

    #[test]
    fn snake_case_rejects_empty() {
        assert_eq!(import_name_case(""), ImportNameCase::NotSnakeCase);
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

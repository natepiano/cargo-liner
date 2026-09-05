use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use proc_macro2::LineColumn;
use quote::quote;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;

use crate::rust_syntax;
use crate::rust_syntax::PathAnchor;

pub(super) struct FlattenedImport {
    pub(super) segments: Vec<String>,
    pub(super) rename:   Option<String>,
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

pub(super) fn leaf_is_module(source_root: &Path, absolute_segments: &[String]) -> bool {
    let mut visited = BTreeSet::new();
    resolves_to_module(source_root, absolute_segments, &mut visited)
}

/// Walks `absolute_segments` from the crate root through file-backed modules,
/// then through inline `mod` blocks and `use` re-exports inside the last
/// file-backed module. `visited` holds the paths already under resolution, so a
/// re-export cycle ends instead of recursing forever.
fn resolves_to_module(
    source_root: &Path,
    absolute_segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> bool {
    if absolute_segments.is_empty() || !visited.insert(absolute_segments.to_vec()) {
        return false;
    }

    let mut dir = source_root.to_path_buf();
    let mut module_file = crate_root_file(source_root);
    let mut module_path: Vec<String> = Vec::new();
    for (index, segment) in absolute_segments.iter().enumerate() {
        let file = dir.join(format!("{segment}.rs"));
        let dir_mod = dir.join(segment).join("mod.rs");
        if file.is_file() {
            module_file = Some(file);
        } else if dir_mod.is_file() {
            module_file = Some(dir_mod);
        } else {
            // No file backs this segment, so the remaining path can only exist
            // as inline `mod` blocks or a `use` re-export inside the last
            // file-backed module.
            let remainder = &absolute_segments[index..];
            return module_file.is_some_and(|module_file| {
                declares_module(source_root, &module_file, &module_path, remainder, visited)
            });
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
    source_root: &Path,
    file: &Path,
    module_path: &[String],
    segments: &[String],
    visited: &mut BTreeSet<Vec<String>>,
) -> bool {
    let Some(first) = segments.first() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(file) else {
        return false;
    };
    if !text.contains(first.as_str()) {
        return false;
    }
    let Ok(syntax) = syn::parse_file(&text) else {
        return false;
    };
    declares_inline_mod_path(&syntax, segments)
        || reexports_module(source_root, &syntax, module_path, segments, visited)
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
    source_root: &Path,
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
                resolves_to_module(source_root, &target, visited)
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
    let bound = flat.rename.as_ref().or_else(|| flat.segments.last())?;
    if bound != name {
        return None;
    }
    match PathAnchor::first(&flat.segments)? {
        PathAnchor::SelfMod => {
            let mut absolute = module_path.to_vec();
            absolute.extend(flat.segments[1..].iter().cloned());
            Some(absolute)
        },
        _ => resolve_to_absolute(&flat.segments, module_path),
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

pub(super) fn extract_visibility_prefix(node: &ItemUse) -> String {
    match &node.vis {
        Visibility::Public(_) => "pub ".to_string(),
        Visibility::Restricted(vis) => {
            let path = &vis.path;
            format!("pub({}) ", quote!(#path))
        },
        Visibility::Inherited => String::new(),
    }
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
                segments.push(name.ident.to_string());
                break Some(FlattenedImport {
                    segments,
                    rename: None,
                });
            },
            UseTree::Rename(rename_tree) => {
                segments.push(rename_tree.ident.to_string());
                break Some(FlattenedImport {
                    segments,
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

pub(super) fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, character) in text.char_indices() {
        if character == '\n' {
            offsets.push(index + 1);
        }
    }
    offsets
}

pub(super) fn offset(line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
}

#[cfg(test)]
mod tests {
    use super::is_snake_case_function_name;
    use super::shorten_module_path;

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
}

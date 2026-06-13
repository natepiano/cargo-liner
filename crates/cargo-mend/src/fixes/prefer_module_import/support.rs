use std::path::Path;

use proc_macro2::LineColumn;
use quote::quote;
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
    if absolute_segments.is_empty() {
        return false;
    }

    let parent_segments = &absolute_segments[..absolute_segments.len() - 1];
    let leaf = &absolute_segments[absolute_segments.len() - 1];

    let mut parent_dir = source_root.to_path_buf();
    for segment in parent_segments {
        parent_dir.push(segment);
    }

    parent_dir.join(format!("{leaf}.rs")).is_file()
        || parent_dir.join(leaf).join("mod.rs").is_file()
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

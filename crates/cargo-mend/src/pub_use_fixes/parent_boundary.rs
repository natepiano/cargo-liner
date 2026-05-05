use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use regex::Regex;
use syn::Item;
use syn::ItemUse;
use syn::UseGroup;
use syn::UsePath;
use syn::UseTree;
use syn::Visibility;
use syn::spanned::Spanned;

use super::validated_plan;
use crate::constants::PATH_KEYWORD_SELF;
use crate::constants::PATH_KEYWORD_SUPER;
use crate::imports::UseFix;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ParentBoundaryKey {
    pub(super) parent_module: PathBuf,
    pub(super) item_start:    usize,
    pub(super) item_end:      usize,
}

pub(super) struct ParentExportResolution {
    pub(super) exported_name:   String,
    pub(super) parent_boundary: ParentBoundaryKey,
}

pub(super) fn build_parent_pub_use_edit_for_exports(
    parent_boundary: &ParentBoundaryKey,
    exports: &[(String, String)],
) -> Result<UseFix> {
    let source = fs::read_to_string(&parent_boundary.parent_module)
        .with_context(|| format!("failed to read {}", parent_boundary.parent_module.display()))?;
    let file = syn::parse_file(&source).context("failed to parse parent module file")?;
    let offsets = validated_plan::line_offsets(&source);
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        let Some(use_prefix) = facade_use_prefix(&item_use.vis) else {
            continue;
        };
        let (start, end) = item_use_byte_range(&source, &offsets, &item_use);
        if start != parent_boundary.item_start || end != parent_boundary.item_end {
            continue;
        }

        let local_exports = locally_used_exports(&source, &item_use, exports)?;
        let replacement = rewrite_parent_pub_use_item_for_exports(
            &item_use,
            exports,
            &local_exports,
            use_prefix,
        )?;
        return Ok(UseFix {
            path: parent_boundary.parent_module.clone(),
            start,
            end,
            replacement,
            import_group: None,
        });
    }

    anyhow::bail!(
        "matching parent `pub use` item not found in {} for span {}..{}",
        parent_boundary.parent_module.display(),
        parent_boundary.item_start,
        parent_boundary.item_end
    )
}

pub(super) fn resolve_parent_pub_use_export(
    source: &str,
    line: usize,
    child_module_name: &str,
    item_name: &str,
) -> Result<Option<ParentExportResolution>> {
    let file = syn::parse_file(source).context("failed to parse parent module file")?;
    let offsets = validated_plan::line_offsets(source);
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if facade_use_prefix(&item_use.vis).is_none() {
            continue;
        }
        let start_line = item_use.span().start().line;
        let end_line = item_use.span().end().line;
        if !(start_line..=end_line).contains(&line) {
            continue;
        }
        if parent_pub_use_exports_item(&item_use.tree, child_module_name, item_name) {
            let (item_start, item_end) = item_use_byte_range(source, &offsets, &item_use);
            return Ok(Some(ParentExportResolution {
                exported_name:   item_name.to_string(),
                parent_boundary: ParentBoundaryKey {
                    parent_module: std::path::PathBuf::new(),
                    item_start,
                    item_end,
                },
            }));
        }
        return Ok(None);
    }
    Ok(None)
}

fn parent_pub_use_exports_item(tree: &UseTree, child_module_name: &str, item_name: &str) -> bool {
    parent_pub_use_exports_item_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn parent_pub_use_exports_item_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            parent_pub_use_exports_item_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = if prefix
                .first()
                .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
            {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            normalized.len() == 1 && normalized[0] == child_module_name && name.ident == item_name
        },
        UseTree::Group(group) => group.items.iter().any(|item| {
            parent_pub_use_exports_item_with_prefix(
                prefix.clone(),
                item,
                child_module_name,
                item_name,
            )
        }),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

fn rewrite_parent_pub_use_item_for_exports(
    item_use: &ItemUse,
    exports: &[(String, String)],
    local_exports: &[(String, String)],
    use_prefix: &str,
) -> Result<String> {
    let mut lines = Vec::new();
    if let Some(rewritten_tree) = remove_exports_from_use_tree(Vec::new(), &item_use.tree, exports)
    {
        lines.extend(render_use_lines(&rewritten_tree, use_prefix)?);
    }
    lines.extend(render_parent_local_use_lines(local_exports));
    Ok(lines.join("\n"))
}

fn facade_use_prefix(vis: &Visibility) -> Option<&'static str> {
    match vis {
        Visibility::Public(_) => Some("pub use"),
        Visibility::Restricted(restricted)
            if restricted.path.segments.len() == 1
                && restricted.path.segments[0].ident == PATH_KEYWORD_SUPER =>
        {
            Some("pub(super) use")
        },
        _ => None,
    }
}

fn remove_exports_from_use_tree(
    prefix: Vec<String>,
    tree: &UseTree,
    exports: &[(String, String)],
) -> Option<UseTree> {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            let rewritten = remove_exports_from_use_tree(next, &path.tree, exports)?;
            Some(UseTree::Path(UsePath {
                ident:        path.ident.clone(),
                colon2_token: path.colon2_token,
                tree:         Box::new(rewritten),
            }))
        },
        UseTree::Name(name) => {
            let normalized = if prefix
                .first()
                .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
            {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            if normalized.len() == 1
                && exports.iter().any(|(child_module_name, item_name)| {
                    normalized[0] == *child_module_name && name.ident == item_name
                })
            {
                None
            } else {
                Some(tree.clone())
            }
        },
        UseTree::Group(group) => {
            let kept_items = group
                .items
                .iter()
                .filter_map(|item| remove_exports_from_use_tree(prefix.clone(), item, exports))
                .collect::<Vec<_>>();
            match kept_items.as_slice() {
                [] => None,
                [only] => Some(only.clone()),
                _ => {
                    let mut punctuated = syn::punctuated::Punctuated::new();
                    for item in kept_items {
                        punctuated.push(item);
                    }
                    Some(UseTree::Group(UseGroup {
                        brace_token: group.brace_token,
                        items:       punctuated,
                    }))
                },
            }
        },
        UseTree::Rename(_) | UseTree::Glob(_) => Some(tree.clone()),
    }
}

fn render_use_lines(tree: &UseTree, use_prefix: &str) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    collect_use_lines(Vec::new(), tree, use_prefix, &mut lines)?;
    Ok(lines)
}

fn collect_use_lines(
    path_prefix: Vec<String>,
    tree: &UseTree,
    use_prefix: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    match tree {
        UseTree::Path(path) => {
            let mut next_prefix = path_prefix;
            next_prefix.push(path.ident.to_string());
            collect_use_lines(next_prefix, &path.tree, use_prefix, lines)
        },
        UseTree::Name(name) => {
            let mut segments = path_prefix;
            segments.push(name.ident.to_string());
            lines.push(format!("{use_prefix} {};", segments.join("::")));
            Ok(())
        },
        UseTree::Rename(rename) => {
            let mut segments = path_prefix;
            segments.push(rename.ident.to_string());
            lines.push(format!(
                "{use_prefix} {} as {};",
                segments.join("::"),
                rename.rename
            ));
            Ok(())
        },
        UseTree::Glob(_) => {
            let rendered_prefix = if path_prefix.is_empty() {
                "*".to_string()
            } else {
                format!("{}::*", path_prefix.join("::"))
            };
            lines.push(format!("{use_prefix} {rendered_prefix};"));
            Ok(())
        },
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_lines(path_prefix.clone(), item, use_prefix, lines)?;
            }
            Ok(())
        },
    }
}

fn render_parent_local_use_lines(exports: &[(String, String)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (child_module, item_name) in exports {
        lines.push(format!("use {child_module}::{item_name};"));
    }
    lines
}

fn item_use_byte_range(source: &str, offsets: &[usize], item_use: &ItemUse) -> (usize, usize) {
    let start = validated_plan::offset(offsets, item_use.span().start());
    let end = source[start..]
        .find(';')
        .map_or(source.len(), |semicolon_offset| {
            start + semicolon_offset + 1
        });
    (start, end)
}

fn locally_used_exports(
    source: &str,
    item_use: &ItemUse,
    exports: &[(String, String)],
) -> Result<Vec<(String, String)>> {
    let offsets = validated_plan::line_offsets(source);
    let (start, end) = item_use_byte_range(source, &offsets, item_use);
    let mut source_without_use = source.to_string();
    source_without_use.replace_range(start..end, "");

    let mut locally_used = Vec::new();
    for (child_module, item_name) in exports {
        let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(item_name)))
            .with_context(|| format!("failed to build local-use regex for {item_name}"))?;
        if pattern.is_match(&source_without_use) {
            locally_used.push((child_module.clone(), item_name.clone()));
        }
    }
    Ok(locally_used)
}

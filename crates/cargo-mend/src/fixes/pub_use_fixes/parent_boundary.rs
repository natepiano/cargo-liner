use std::fs;
use std::iter;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;
use syn::parse_file;
use syn::spanned::Spanned;

use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::SourceLines;
use crate::fixes::imports::UseFix;
use crate::rust_syntax;
use crate::rust_syntax::PathAnchor;

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
    let file = parse_file(&source).context("failed to parse parent module file")?;
    let lines = SourceLines::new(&source);
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        let Some(use_prefix) = facade_use_prefix(&item_use.vis) else {
            continue;
        };
        let (start, end) = facade_redirect::item_use_byte_range(&lines, &item_use);
        if start != parent_boundary.item_start || end != parent_boundary.item_end {
            continue;
        }

        let local_exports = locally_used_exports(&source, (start, end), exports);
        let replacement =
            rewrite_parent_pub_use_item_for_exports(&item_use, exports, &local_exports, use_prefix);
        return Ok(UseFix {
            path: parent_boundary.parent_module.clone(),
            start,
            end,
            replacement,
            import_group: None,
        });
    }

    bail!(
        "matching parent re-export item not found in {} for span {}..{}",
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
    let file = parse_file(source).context("failed to parse parent module file")?;
    let lines = SourceLines::new(source);
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
        if !parent_pub_use_exports_item(&item_use.tree, child_module_name, item_name) {
            // Two `use` declarations can share one physical line, so keep
            // scanning the rest of the line rather than treating the first
            // textual match as authoritative.
            continue;
        }
        let (item_start, item_end) = facade_redirect::item_use_byte_range(&lines, &item_use);
        return Ok(Some(ParentExportResolution {
            exported_name:   item_name.to_string(),
            parent_boundary: ParentBoundaryKey {
                parent_module: PathBuf::new(),
                item_start,
                item_end,
            },
        }));
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
            let normalized = rust_syntax::trim_leading_self(&prefix);
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
) -> String {
    let mut lines = Vec::new();
    if let Some(rewritten_tree) =
        facade_redirect::remove_use_leaves(Vec::new(), &item_use.tree, &|prefix, name| {
            let normalized = rust_syntax::trim_leading_self(prefix);
            normalized.len() == 1
                && exports.iter().any(|(child_module_name, item_name)| {
                    normalized[0] == *child_module_name && name == item_name
                })
        })
    {
        lines.extend(facade_redirect::render_use_lines(
            &rewritten_tree,
            use_prefix,
        ));
    }
    lines.extend(render_parent_local_use_lines(local_exports));
    lines.join("\n")
}

pub(crate) fn facade_use_prefix(vis: &Visibility) -> Option<&'static str> {
    match vis {
        Visibility::Public(_) => Some("pub use"),
        Visibility::Restricted(restricted) if restricted.path.segments.len() == 1 => {
            let path_anchor =
                PathAnchor::from(restricted.path.segments[0].ident.to_string().as_str());
            match path_anchor {
                PathAnchor::Super => Some("pub(super) use"),
                PathAnchor::Crate
                | PathAnchor::SelfMod
                | PathAnchor::SelfType
                | PathAnchor::Name => None,
            }
        },
        _ => None,
    }
}

fn render_parent_local_use_lines(exports: &[(String, String)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (child_module, item_name) in exports {
        lines.push(format!("use {child_module}::{item_name};"));
    }
    lines
}

fn locally_used_exports(
    source: &str,
    item_use_range: (usize, usize),
    exports: &[(String, String)],
) -> Vec<(String, String)> {
    let (start, end) = item_use_range;
    let mut locally_used = Vec::new();
    for (child_module, item_name) in exports {
        if facade_redirect::word_occurs_outside(source, iter::once(start..end), item_name) {
            locally_used.push((child_module.clone(), item_name.clone()));
        }
    }
    locally_used
}

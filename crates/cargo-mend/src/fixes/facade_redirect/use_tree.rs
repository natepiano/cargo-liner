use syn::Ident;
use syn::UseGroup;
use syn::UsePath;
use syn::UseTree;
use syn::punctuated::Punctuated;

/// `tree` written back as source, one line, groups as `{a, b}`.
pub(in crate::fixes) fn render_use_tree(tree: &UseTree) -> String {
    match tree {
        UseTree::Path(path) => format!("{}::{}", path.ident, render_use_tree(&path.tree)),
        UseTree::Name(name) => name.ident.to_string(),
        UseTree::Rename(rename) => format!("{} as {}", rename.ident, rename.rename),
        UseTree::Glob(_) => "*".to_string(),
        UseTree::Group(group) => format!(
            "{{{}}}",
            group
                .items
                .iter()
                .map(render_use_tree)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `tree` without the `Name` leaves `remove` selects, given each leaf's
/// written prefix segments and identifier. A group left with one item
/// collapses to it; a tree left with nothing is `None`.
pub(in crate::fixes) fn remove_use_leaves(
    prefix: Vec<String>,
    tree: &UseTree,
    remove: &impl Fn(&[String], &Ident) -> bool,
) -> Option<UseTree> {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            let rewritten = remove_use_leaves(next, &path.tree, remove)?;
            Some(UseTree::Path(UsePath {
                ident:        path.ident.clone(),
                colon2_token: path.colon2_token,
                tree:         Box::new(rewritten),
            }))
        },
        UseTree::Name(name) => (!remove(&prefix, &name.ident)).then(|| tree.clone()),
        UseTree::Group(group) => {
            let kept_items = group
                .items
                .iter()
                .filter_map(|item| remove_use_leaves(prefix.clone(), item, remove))
                .collect::<Vec<_>>();
            match kept_items.as_slice() {
                [] => None,
                [only] => Some(only.clone()),
                _ => Some(UseTree::Group(UseGroup {
                    brace_token: group.brace_token,
                    items:       kept_items.into_iter().collect::<Punctuated<_, _>>(),
                })),
            }
        },
        UseTree::Rename(_) | UseTree::Glob(_) => Some(tree.clone()),
    }
}

/// `tree` flattened to one `{use_prefix} path;` line per leaf.
pub(in crate::fixes) fn render_use_lines(tree: &UseTree, use_prefix: &str) -> Vec<String> {
    let mut lines = Vec::new();
    collect_use_lines(Vec::new(), tree, use_prefix, &mut lines);
    lines
}

fn collect_use_lines(
    path_prefix: Vec<String>,
    tree: &UseTree,
    use_prefix: &str,
    lines: &mut Vec<String>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next_prefix = path_prefix;
            next_prefix.push(path.ident.to_string());
            collect_use_lines(next_prefix, &path.tree, use_prefix, lines);
        },
        UseTree::Name(name) => {
            let mut segments = path_prefix;
            segments.push(name.ident.to_string());
            lines.push(format!("{use_prefix} {};", segments.join("::")));
        },
        UseTree::Rename(rename) => {
            let mut segments = path_prefix;
            segments.push(rename.ident.to_string());
            lines.push(format!(
                "{use_prefix} {} as {};",
                segments.join("::"),
                rename.rename
            ));
        },
        UseTree::Glob(_) => {
            let rendered_prefix = if path_prefix.is_empty() {
                "*".to_string()
            } else {
                format!("{}::*", path_prefix.join("::"))
            };
            lines.push(format!("{use_prefix} {rendered_prefix};"));
        },
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_lines(path_prefix.clone(), item, use_prefix, lines);
            }
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::ItemUse;
    use syn::UseTree;
    use syn::parse_str;

    use super::remove_use_leaves;
    use super::render_use_lines;
    use super::render_use_tree;

    fn tree(source: &str) -> UseTree { parse_str::<ItemUse>(source).expect("parse use").tree }

    #[test]
    fn removing_one_of_two_group_leaves_collapses_the_group() {
        let kept = remove_use_leaves(Vec::new(), &tree("use a::{b::X, Y};"), &|prefix, name| {
            prefix == ["a", "b"] && name == "X"
        })
        .expect("a leaf is left");
        assert_eq!(render_use_tree(&kept), "a::Y");
    }

    #[test]
    fn removing_every_leaf_leaves_nothing() {
        assert!(remove_use_leaves(Vec::new(), &tree("use a::X;"), &|_, _| true).is_none());
    }

    #[test]
    fn a_nested_group_flattens_to_one_line_per_leaf() {
        assert_eq!(
            render_use_lines(&tree("use a::{b::{X, Y as Z}, *};"), "pub use"),
            ["pub use a::b::X;", "pub use a::b::Y as Z;", "pub use a::*;"]
        );
    }
}

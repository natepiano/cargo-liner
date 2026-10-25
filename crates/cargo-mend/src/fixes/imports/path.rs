use proc_macro2::LineColumn;
use syn::UseTree;

use crate::config::DiagnosticCode;
use crate::rust_syntax;
use crate::rust_syntax::PathAnchor;

pub(super) struct ImportCandidate {
    pub(super) original:        String,
    pub(super) replacement:     String,
    pub(super) diagnostic_code: DiagnosticCode,
    pub(super) message:         &'static str,
}

struct FlattenedImport {
    segments: Vec<String>,
    original: String,
    rename:   Option<String>,
}

pub(super) fn analyze_use_tree(
    current_module_path: &[String],
    tree: &UseTree,
) -> Option<ImportCandidate> {
    let import = flatten_use_tree(tree)?;
    match PathAnchor::first(&import.segments)? {
        PathAnchor::Crate => {},
        PathAnchor::Super | PathAnchor::SelfMod | PathAnchor::SelfType | PathAnchor::Name => {
            return None;
        },
    }

    let target_segments = &import.segments[1..];
    if target_segments.len() < 2 {
        return None;
    }

    let relation = LocalImportRelation::for_paths(current_module_path, target_segments);
    let relative = relation.render(import.rename.as_deref())?;
    if relative == import.original {
        return None;
    }

    Some(ImportCandidate {
        original:        import.original,
        replacement:     relative,
        diagnostic_code: DiagnosticCode::ShortenLocalCrateImport,
        message:         "it stays within the same local module boundary",
    })
}

pub(super) fn analyze_deep_super(
    current_module_path: &[String],
    tree: &UseTree,
) -> Option<ImportCandidate> {
    let import = flatten_use_tree(tree)?;
    let super_count = rust_syntax::leading_super_count(&import.segments);
    if super_count < 2 {
        return None;
    }
    if super_count > current_module_path.len() {
        return None;
    }

    let ancestor_path = &current_module_path[..current_module_path.len() - super_count];
    let remaining = &import.segments[super_count..];
    let mut replacement_segments = vec!["crate".to_string()];
    replacement_segments.extend(ancestor_path.iter().cloned());
    replacement_segments.extend(remaining.iter().cloned());
    let replacement = format_path(&replacement_segments, import.rename.as_deref());

    Some(ImportCandidate {
        original: import.original,
        replacement,
        diagnostic_code: DiagnosticCode::ReplaceDeepSuperImport,
        message: "deep `super::` chain is hard to follow — use a named `crate::` path",
    })
}

fn flatten_use_tree(tree: &UseTree) -> Option<FlattenedImport> {
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
                let original = format_path(&segments, None);
                break Some(FlattenedImport {
                    segments,
                    original,
                    rename: None,
                });
            },
            UseTree::Rename(rename_tree) => {
                segments.push(rename_tree.ident.to_string());
                let rename = rename_tree.rename.to_string();
                let original = format_path(&segments, Some(&rename));
                break Some(FlattenedImport {
                    segments,
                    original,
                    rename: Some(rename),
                });
            },
            _ => break None,
        }
    }
}

/// Local boundaries within which a crate-qualified import can be shortened.
enum LocalImportRelation<'a> {
    Descendant { suffix: &'a [String] },
    ParentSibling { suffix: &'a [String] },
    OutsideBoundary,
}

impl<'a> LocalImportRelation<'a> {
    fn for_paths(current: &[String], target: &'a [String]) -> Self {
        let common = common_prefix_len(current, target);
        if common == 0 {
            return Self::OutsideBoundary;
        }
        match current.len().saturating_sub(common) {
            0 => Self::Descendant {
                suffix: &target[common..],
            },
            1 if common < target.len() => Self::ParentSibling {
                suffix: &target[common..],
            },
            _ => Self::OutsideBoundary,
        }
    }

    fn render(&self, rename: Option<&str>) -> Option<String> {
        match self {
            Self::Descendant { suffix } => Some(format_path(suffix, rename)),
            Self::ParentSibling { suffix } => {
                let segments = std::iter::once("super".to_string())
                    .chain(suffix.iter().cloned())
                    .collect::<Vec<_>>();
                Some(format_path(&segments, rename))
            },
            Self::OutsideBoundary => None,
        }
    }
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(l, r)| l == r)
        .count()
}

fn format_path(segments: &[String], rename: Option<&str>) -> String {
    let mut path = segments.join("::");
    if let Some(rename) = rename {
        path.push_str(" as ");
        path.push_str(rename);
    }
    path
}

pub(super) fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::ItemUse;

    use super::analyze_use_tree;

    fn module(path: &str) -> Vec<String> {
        path.split("::")
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn local_imports_shorten_only_within_the_local_boundary() {
        for (current, source, expected) in [
            ("a", "use crate::a::b::Item;", Some("b::Item")),
            (
                "a::c",
                "use crate::a::b::Item as Alias;",
                Some("super::b::Item as Alias"),
            ),
            ("a::b", "use crate::a::b;", Some("")),
            ("a::b::c", "use crate::a::b;", None),
            ("a::b::c", "use crate::a::b as Parent;", None),
            ("a::b::c", "use crate::a::Item;", None),
            ("a", "use crate::x::Item;", None),
            ("", "use crate::a::Item;", None),
            ("a", "use crate::a;", None),
            ("a", "use self::b::Item;", None),
            ("a", "use crate::a::{First, Second};", None),
        ] {
            let item: ItemUse = syn::parse_str(source).expect("parse use");
            assert_eq!(
                analyze_use_tree(&module(current), &item.tree)
                    .map(|candidate| candidate.replacement)
                    .as_deref(),
                expected,
                "{current}: {source}"
            );
        }
    }
}

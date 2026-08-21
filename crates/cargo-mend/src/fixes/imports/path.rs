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

    let current_len = current_module_path.len();
    let common = common_prefix_len(current_module_path, target_segments);
    if common == 0 {
        return None;
    }
    let up_count = current_len.saturating_sub(common);
    if up_count > 1 {
        return None;
    }

    let relative = build_relative_path(current_module_path, target_segments, &import)?;
    if relative == import.original
        || !(relative.starts_with("super::") || target_segments.starts_with(current_module_path))
    {
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

fn build_relative_path(
    current_module_path: &[String],
    target_segments: &[String],
    import: &FlattenedImport,
) -> Option<String> {
    let common = common_prefix_len(current_module_path, target_segments);
    let up_count = current_module_path.len().saturating_sub(common);
    let mut relative_segments = Vec::new();
    if up_count > 1 {
        return None;
    }
    if up_count == 1 {
        relative_segments.push("super".to_string());
    }
    relative_segments.extend(target_segments[common..].iter().cloned());
    Some(format_path(&relative_segments, import.rename.as_deref()))
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

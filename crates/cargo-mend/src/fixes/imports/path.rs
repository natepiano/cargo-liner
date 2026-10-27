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

#[cfg(test)]
thread_local! {
    static INDEX_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One source and its own line index, used to check external span coordinates.
pub(in crate::fixes) struct ImportSource<'a> {
    text:    &'a str,
    offsets: Vec<usize>,
}

/// An addressable source position, including the real line used in diagnostics.
struct SourcePosition<'a> {
    offset:      usize,
    source_line: &'a str,
}

/// A complete import span checked against the source that supplies its text.
pub(in crate::fixes) struct ImportSourceLocation<'a> {
    start:       usize,
    end:         usize,
    source_line: &'a str,
    text:        &'a str,
}

impl<'a> ImportSourceLocation<'a> {
    pub(in crate::fixes) const fn start(&self) -> usize { self.start }

    pub(in crate::fixes) const fn end(&self) -> usize { self.end }

    pub(in crate::fixes) const fn source_line(&self) -> &'a str { self.source_line }

    pub(super) const fn text(&self) -> &'a str { self.text }
}

impl<'a> ImportSource<'a> {
    pub(in crate::fixes) fn new(text: &'a str) -> Self {
        #[cfg(test)]
        INDEX_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        let mut offsets = vec![0];
        offsets.extend(text.match_indices('\n').map(|(offset, _)| offset + 1));
        Self { text, offsets }
    }

    #[cfg(test)]
    pub(in crate::fixes) fn construction_count() -> usize {
        INDEX_CONSTRUCTIONS.with(std::cell::Cell::get)
    }

    fn position(&self, position: LineColumn) -> Option<SourcePosition<'a>> {
        let start = *self.offsets.get(position.line.checked_sub(1)?)?;
        let rest = &self.text[start..];
        let (line, source_line) = match rest.split_once('\n') {
            Some((line, _)) => (line, line.strip_suffix('\r').unwrap_or(line)),
            None => (rest, rest),
        };
        let column = line
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(line.len()))
            .nth(position.column)?;
        Some(SourcePosition {
            offset: start + column,
            source_line,
        })
    }

    pub(in crate::fixes) fn locate(
        &self,
        start: LineColumn,
        end: LineColumn,
    ) -> Option<ImportSourceLocation<'a>> {
        let start = self.position(start)?;
        let end = self.position(end)?;
        let text = self.text.get(start.offset..end.offset)?;
        Some(ImportSourceLocation {
            start: start.offset,
            end: end.offset,
            source_line: start.source_line,
            text,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use proc_macro2::LineColumn;
    use syn::ItemUse;

    use super::analyze_use_tree;

    fn module(path: &str) -> Vec<String> {
        path.split("::")
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn import_locations_require_real_lines_columns_and_ordered_spans() {
        let source = super::ImportSource::new("// é\nuse é::Item;\n");
        let at = |line, column| LineColumn { line, column };
        let located = source
            .locate(at(2, 4), at(2, 11))
            .expect("valid UTF-8 span");
        assert_eq!(located.text, "é::Item");
        assert_eq!(located.source_line, "use é::Item;");
        for (start, end) in [
            (at(0, 0), at(1, 0)),
            (at(4, 0), at(4, 0)),
            (at(2, 99), at(2, 99)),
            (at(2, 5), at(2, 4)),
        ] {
            assert!(source.locate(start, end).is_none());
        }
        assert!(source.locate(at(3, 0), at(3, 0)).is_some());
    }

    #[test]
    fn source_lines_preserve_unterminated_carriage_returns() {
        let at = |line, column| LineColumn { line, column };
        for (text, line, expected) in [
            ("use crate::a::X;\r", 1, "use crate::a::X;\r"),
            ("use crate::a::X;\r\n", 1, "use crate::a::X;"),
            ("use crate::a::X;\r\n", 2, ""),
        ] {
            let source = super::ImportSource::new(text);
            let located = source.locate(at(line, 0), at(line, 0)).expect("valid line");
            assert_eq!(located.source_line(), expected);
        }
        let source = super::ImportSource::new("é\r\n");
        let located = source
            .locate(at(1, 0), at(1, 2))
            .expect("physical CR column");
        assert_eq!(located.text(), "é\r");
        assert_eq!(located.source_line(), "é");
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

use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use proc_macro2::Delimiter;
use proc_macro2::Group;
use proc_macro2::LineColumn;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::ItemUse;
use syn::ext::IdentExt;
use syn::spanned::Spanned;

use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::UseFix;

/// Byte offsets of the lines of one source text, for turning syn span
/// positions into the byte ranges a `UseFix` edits.
pub(in crate::fixes) struct SourceLines<'a> {
    source:      &'a str,
    line_starts: Vec<usize>,
}

/// A source line whose index cannot underflow; membership in this source is
/// still checked by line lookup.
#[derive(Clone, Copy)]
struct OneBasedLine(NonZeroUsize);

impl OneBasedLine {
    fn new(line: usize) -> Option<Self> { NonZeroUsize::new(line).map(Self) }
}

/// An addressable character position in one source, represented by its borrowed
/// prefix. Only `SourceLines::position` constructs it; conversion cannot apply
/// its coordinates to a different source.
#[derive(Clone, Copy)]
pub(in crate::fixes) struct SourcePosition<'a> {
    prefix: &'a str,
}

/// Why an external span cannot address this source. Invalid coordinates never
/// become edits at byte zero or EOF, or continue into a later physical line.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::fixes) enum SourcePositionError {
    LineZero,
    UnknownLine { line: usize },
    ColumnPastLineEnd { line: usize, column: usize },
    ReversedSpan { start: usize, end: usize },
}

impl Display for SourcePositionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineZero => formatter.write_str("source line must be one-based"),
            Self::UnknownLine { line } => write!(formatter, "source line {line} does not exist"),
            Self::ColumnPastLineEnd { line, column } => {
                write!(
                    formatter,
                    "source column {column} is past the end of line {line}"
                )
            },
            Self::ReversedSpan { start, end } => {
                write!(
                    formatter,
                    "source span ends at {end} before its start at {start}"
                )
            },
        }
    }
}

impl Error for SourcePositionError {}

impl<'a> SourceLines<'a> {
    pub(in crate::fixes) fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (index, character) in source.char_indices() {
            if character == '\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    /// Checks an external span position against this source's physical line.
    /// `LineColumn` counts characters, so columns are walked rather than added
    /// as bytes. The line-end position is valid, including an empty final line.
    pub(in crate::fixes) fn position(
        &self,
        position: LineColumn,
    ) -> Result<SourcePosition<'a>, SourcePositionError> {
        let line = OneBasedLine::new(position.line).ok_or(SourcePositionError::LineZero)?;
        let start =
            *self
                .line_starts
                .get(line.0.get() - 1)
                .ok_or(SourcePositionError::UnknownLine {
                    line: position.line,
                })?;
        let rest = &self.source[start..];
        let physical_line = rest.split_once('\n').map_or(rest, |(line, _)| line);
        let column = physical_line
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(physical_line.len()))
            .nth(position.column)
            .ok_or(SourcePositionError::ColumnPastLineEnd {
                line:   position.line,
                column: position.column,
            })?;
        Ok(SourcePosition {
            prefix: &self.source[..start + column],
        })
    }

    /// Converts a checked position from its own source into an edit offset.
    pub(in crate::fixes) const fn offset(position: SourcePosition<'_>) -> usize {
        position.prefix.len()
    }

    /// Validates both external endpoints before constructing an ordered range.
    pub(in crate::fixes) fn span_range(
        &self,
        start: LineColumn,
        end: LineColumn,
    ) -> Result<(usize, usize), SourcePositionError> {
        let start = Self::offset(self.position(start)?);
        let end = Self::offset(self.position(end)?);
        if start > end {
            return Err(SourcePositionError::ReversedSpan { start, end });
        }
        Ok((start, end))
    }

    /// The byte range of 1-based `line`, trailing newline included.
    pub(in crate::fixes) fn line_span(&self, line: usize) -> Option<(usize, usize)> {
        self.checked_line_span(OneBasedLine::new(line)?)
    }

    fn checked_line_span(&self, line: OneBasedLine) -> Option<(usize, usize)> {
        let line = line.0.get();
        let start = *self.line_starts.get(line - 1)?;
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len());
        Some((start, end))
    }

    /// The whitespace between the start of the line holding `offset` and
    /// `offset`, or an empty string when code precedes it on that line.
    pub(in crate::fixes) fn indent_before(&self, offset: usize) -> &'a str {
        let line_start = self.source[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let leading = &self.source[line_start..offset];
        if leading.chars().all(char::is_whitespace) {
            leading
        } else {
            ""
        }
    }
}

/// The bytes a `use` item occupies: its outer attributes through its `;`.
pub(in crate::fixes) fn item_use_byte_range(
    lines: &SourceLines<'_>,
    item_use: &ItemUse,
) -> Result<(usize, usize), SourcePositionError> {
    lines.span_range(item_use.span().start(), item_use.semi_token.span.end())
}

/// A deletion may take whole lines only when no code shares them with the item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletionExtent {
    WholeLines,
    ItemOnly,
}

impl DeletionExtent {
    fn at(source: &str, range: &Range<usize>, line_start: usize, line_end: usize) -> Self {
        if source[line_start..range.start].trim().is_empty()
            && source[range.end..line_end].trim().is_empty()
        {
            Self::WholeLines
        } else {
            Self::ItemOnly
        }
    }
}

/// A neighboring physical line classified together with its byte range.
/// Blank neighbors may be consumed to avoid leaving duplicate or edge blank lines.
#[derive(Debug, PartialEq, Eq)]
enum NeighborLine {
    Missing,
    Blank { range: Range<usize> },
    Occupied { range: Range<usize> },
}

impl NeighborLine {
    fn at(source: &str, range: Option<Range<usize>>) -> Self {
        range.map_or(Self::Missing, |range| {
            if source[range.clone()].trim().is_empty() {
                Self::Blank { range }
            } else {
                Self::Occupied { range }
            }
        })
    }
}

/// A fix removing `range`, widened to whole lines when nothing else shares them.
pub(in crate::fixes) fn line_deletion(file: &Path, source: &str, range: Range<usize>) -> UseFix {
    let line_start = source[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[range.end..]
        .find('\n')
        .map_or(source.len(), |index| range.end + index + 1);
    let (start, end) = match DeletionExtent::at(source, &range, line_start, line_end) {
        DeletionExtent::WholeLines => with_neighboring_blank_line(source, line_start, line_end),
        DeletionExtent::ItemOnly => (range.start, range.end),
    };
    UseFix {
        path: file.to_path_buf(),
        start,
        end,
        replacement: String::new(),
        import_group: None,
    }
}

/// Whole lines `line_start..line_end`, grown by one blank line so that
/// deleting them leaves no blank line at the file's start or end and no two
/// blank lines in a row: the blank line after them when the file's start or a
/// blank line comes before them, or the blank line before them when they end
/// the file.
fn with_neighboring_blank_line(source: &str, line_start: usize, line_end: usize) -> (usize, usize) {
    let before = &source[..line_start];
    let previous_line = before
        .strip_suffix('\n')
        .map(|text| text.rfind('\n').map_or(0, |index| index + 1)..line_start);
    let next_line = source[line_end..]
        .find('\n')
        .map(|index| line_end..line_end + index + 1);
    match (
        NeighborLine::at(source, previous_line),
        NeighborLine::at(source, next_line),
    ) {
        (NeighborLine::Missing | NeighborLine::Blank { .. }, NeighborLine::Blank { range }) => {
            (line_start, range.end)
        },
        (NeighborLine::Blank { range }, NeighborLine::Missing) => {
            if line_end == source.len() {
                (range.start, line_end)
            } else {
                (line_start, line_end)
            }
        },
        (NeighborLine::Missing, NeighborLine::Missing | NeighborLine::Occupied { .. })
        | (NeighborLine::Blank { .. }, NeighborLine::Occupied { .. })
        | (
            NeighborLine::Occupied { .. },
            NeighborLine::Missing | NeighborLine::Blank { .. } | NeighborLine::Occupied { .. },
        ) => (line_start, line_end),
    }
}

/// Identifier use outside excluded ranges. Unparseable text conservatively keeps the name,
/// just like an observed identifier, so a failed lex never removes a needed import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::fixes) enum IdentifierUse {
    Absent,
    Present,
    Unparseable,
}

/// Identifier or inline-format-argument presence, excluding documentation attribute bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentifierPresence {
    Absent,
    Present,
}

/// Documentation bodies lex as bracketed groups starting with `doc`; skip their contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributeGroupKind {
    Documentation,
    Traversable,
}

/// Whether `word` occurs as an identifier in `text` once every `excluded`
/// byte range is blanked out. Comments, doc comments, and string literals do
/// not count, except a `{word}` or `{word:` inline format argument in a
/// literal. Text that does not lex counts as an occurrence, so the caller keeps
/// the name.
pub(in crate::fixes) fn word_occurs_outside(
    text: &str,
    excluded: impl IntoIterator<Item = Range<usize>>,
    word: &str,
) -> IdentifierUse {
    let mut blanked = text.to_string();
    for range in excluded {
        let blank = " ".repeat(range.len());
        blanked.replace_range(range, &blank);
    }
    blanked
        .parse::<TokenStream>()
        .map_or(IdentifierUse::Unparseable, |tokens| {
            match tokens_name(tokens, word) {
                IdentifierPresence::Absent => IdentifierUse::Absent,
                IdentifierPresence::Present => IdentifierUse::Present,
            }
        })
}

/// Whether `tokens` hold `word` as an identifier or as an inline format
/// argument, skipping `#[doc = ...]` attribute bodies.
fn tokens_name(tokens: TokenStream, word: &str) -> IdentifierPresence {
    for token in tokens {
        let presence = match token {
            TokenTree::Ident(ident) => {
                if ident.unraw() == word {
                    IdentifierPresence::Present
                } else {
                    IdentifierPresence::Absent
                }
            },
            TokenTree::Group(group) => match is_doc_attribute(&group) {
                AttributeGroupKind::Documentation => IdentifierPresence::Absent,
                AttributeGroupKind::Traversable => tokens_name(group.stream(), word),
            },
            TokenTree::Literal(literal) => {
                let text = literal.to_string();
                if text.contains(&format!("{{{word}}}")) || text.contains(&format!("{{{word}:")) {
                    IdentifierPresence::Present
                } else {
                    IdentifierPresence::Absent
                }
            },
            TokenTree::Punct(_) => IdentifierPresence::Absent,
        };
        match presence {
            IdentifierPresence::Present => return presence,
            IdentifierPresence::Absent => {},
        }
    }
    IdentifierPresence::Absent
}

/// Whether `group` is the bracketed body of a `#[doc = ...]` attribute, which
/// is how a doc comment lexes.
fn is_doc_attribute(group: &Group) -> AttributeGroupKind {
    match group.delimiter() {
        Delimiter::Bracket => match group.stream().into_iter().next() {
            Some(TokenTree::Ident(ident)) => {
                if ident == "doc" {
                    AttributeGroupKind::Documentation
                } else {
                    AttributeGroupKind::Traversable
                }
            },
            Some(TokenTree::Group(_) | TokenTree::Literal(_) | TokenTree::Punct(_)) | None => {
                AttributeGroupKind::Traversable
            },
        },
        Delimiter::Parenthesis | Delimiter::Brace | Delimiter::None => {
            AttributeGroupKind::Traversable
        },
    }
}

pub(in crate::fixes) fn find_source_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(OsStr::to_str) == Some(SOURCE_DIR_SRC))
        .map(Path::to_path_buf)
}

pub(in crate::fixes) fn dedup_fixes(fixes: &mut Vec<UseFix>) {
    let mut seen = BTreeSet::new();
    fixes.retain(|fix| {
        seen.insert((
            fix.path.clone(),
            fix.start,
            fix.end,
            fix.replacement.clone(),
        ))
    });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::iter;
    use std::path::Path;

    use proc_macro2::Delimiter;
    use proc_macro2::Group;
    use proc_macro2::LineColumn;
    use syn::Item;
    use syn::parse_file;

    use super::AttributeGroupKind;
    use super::DeletionExtent;
    use super::IdentifierPresence;
    use super::IdentifierUse;
    use super::NeighborLine;
    use super::SourceLines;
    use super::SourcePositionError;
    use super::is_doc_attribute;
    use super::item_use_byte_range;
    use super::line_deletion;
    use super::tokens_name;
    use super::word_occurs_outside;

    /// The text left after `line_deletion` removes the first occurrence of
    /// `target` from `source`.
    fn after_line_deletion(source: &str, target: &str) -> String {
        let start = source.find(target).expect("target in source");
        let fix = line_deletion(Path::new("lib.rs"), source, start..start + target.len());
        format!("{}{}", &source[..fix.start], &source[fix.end..])
    }

    #[test]
    fn external_positions_reject_invalid_lines_and_columns() {
        let lines = SourceLines::new("é\nx\n");
        let at = |line, column| LineColumn { line, column };
        for (raw, error) in [
            (at(0, 0), SourcePositionError::LineZero),
            (at(4, 0), SourcePositionError::UnknownLine { line: 4 }),
            (
                at(1, 2),
                SourcePositionError::ColumnPastLineEnd {
                    line:   1,
                    column: 2,
                },
            ),
            (
                at(2, 2),
                SourcePositionError::ColumnPastLineEnd {
                    line:   2,
                    column: 2,
                },
            ),
        ] {
            assert_eq!(lines.position(raw).map(SourceLines::offset), Err(error));
        }
        assert_eq!(lines.position(at(1, 1)).map(SourceLines::offset), Ok(2));
        assert_eq!(lines.position(at(3, 0)).map(SourceLines::offset), Ok(5));
        assert_eq!(
            lines.span_range(at(2, 0), at(1, 0)),
            Err(SourcePositionError::ReversedSpan { start: 3, end: 0 })
        );
    }

    #[test]
    fn line_spans_reject_zero_and_unknown_lines() {
        let lines = SourceLines::new("a\nb\n");
        assert_eq!(lines.line_span(0), None);
        assert_eq!(lines.line_span(1), Some((0, 2)));
        assert_eq!(lines.line_span(2), Some((2, 4)));
        assert_eq!(lines.line_span(3), Some((4, 4)));
        assert_eq!(lines.line_span(4), None);
    }

    #[test]
    fn a_deleted_first_line_takes_the_blank_line_after_it() {
        assert_eq!(
            after_line_deletion("use a::b;\n\nfn f() {}\n", "use a::b;"),
            "fn f() {}\n"
        );
    }

    #[test]
    fn a_deleted_line_between_blank_lines_leaves_one_blank_line() {
        assert_eq!(
            after_line_deletion("mod m;\n\nuse a::b;\n\nfn f() {}\n", "use a::b;"),
            "mod m;\n\nfn f() {}\n"
        );
    }

    #[test]
    fn a_deleted_last_line_takes_the_blank_line_before_it() {
        assert_eq!(
            after_line_deletion("mod m;\n\nuse a::b;\n", "use a::b;"),
            "mod m;\n"
        );
    }

    #[test]
    fn a_deleted_line_between_code_lines_takes_no_blank_line() {
        assert_eq!(
            after_line_deletion("mod m;\nuse a::b;\n\nfn f() {}\n", "use a::b;"),
            "mod m;\n\nfn f() {}\n"
        );
    }

    #[test]
    fn a_column_past_a_multibyte_character_lands_on_the_right_byte() {
        let source = "// é\nlet é = x;\n";
        let lines = SourceLines::new(source);
        let position = lines
            .position(LineColumn {
                line:   2,
                column: 8,
            })
            .expect("valid column");
        let offset = SourceLines::offset(position);
        assert_eq!(&source[offset..], "x;\n");
    }

    #[test]
    fn a_use_range_ends_at_its_own_semicolon_past_an_attribute_semicolon() {
        let source = "#[doc = \"a; b\"]\npub use crate::x::Y;\nfn f() {}\n";
        let file = parse_file(source).expect("parse");
        let item_use = file
            .items
            .iter()
            .find_map(|item| match item {
                Item::Use(item_use) => Some(item_use),
                _ => None,
            })
            .expect("use item");
        let (start, end) = item_use_byte_range(&SourceLines::new(source), item_use).expect("range");
        assert_eq!(
            &source[start..end],
            "#[doc = \"a; b\"]\npub use crate::x::Y;"
        );
    }

    #[test]
    fn a_blanked_range_hides_its_words() {
        let text = "use a::Name;\nfn f() {}\n";
        assert_eq!(
            word_occurs_outside(text, iter::once(0..12), "Name"),
            IdentifierUse::Absent
        );
        assert_eq!(
            word_occurs_outside(text, iter::empty(), "Name"),
            IdentifierUse::Present
        );
    }

    #[test]
    fn comments_and_string_literals_do_not_name_the_word() {
        let text = "//! Builds a [`Name`].\n/// A `Name`.\nfn f() -> &'static str {\n    // Name\n    \"Name\"\n}\n";
        assert_eq!(
            word_occurs_outside(text, iter::empty(), "Name"),
            IdentifierUse::Absent
        );
    }

    #[test]
    fn an_inline_format_argument_or_raw_identifier_names_the_word() {
        // Split `format_call` so the test's own literal holds no format argument.
        let format_call = ["fn f() -> String { format!(\"{", "Name:?}\") }\n"].concat();
        assert_eq!(
            word_occurs_outside(&format_call, iter::empty(), "Name"),
            IdentifierUse::Present
        );
        assert_eq!(
            word_occurs_outside(concat!("\"{", "Name}\""), iter::empty(), "Name"),
            IdentifierUse::Present
        );
        assert_eq!(
            word_occurs_outside("fn f() { r#Name(); }\n", iter::empty(), "Name"),
            IdentifierUse::Present
        );
    }

    #[test]
    fn text_that_does_not_lex_names_the_word() {
        assert_eq!(
            word_occurs_outside("fn f() { \"open\n", iter::empty(), "Name"),
            IdentifierUse::Unparseable
        );
    }

    #[test]
    fn indent_is_empty_when_code_precedes_the_offset() {
        let source = "mod m {\n    use a::b;\n}\nfn f() { use c::d; }\n";
        let lines = SourceLines::new(source);
        let inner = source.find("use a").expect("inner use");
        let inline = source.find("use c").expect("inline use");
        assert_eq!(lines.indent_before(inner), "    ");
        assert_eq!(lines.indent_before(inline), "");
    }
    #[test]
    fn deletion_extent_keeps_code_sharing_either_side_of_the_item() {
        for (source, expected) in [
            ("  use a::b;  ", DeletionExtent::WholeLines),
            ("mod m {} use a::b;", DeletionExtent::ItemOnly),
            ("use a::b; fn f() {}", DeletionExtent::ItemOnly),
        ] {
            let start = source.find("use").expect("use start");
            let end = source.find(';').expect("use end") + 1;
            assert_eq!(
                DeletionExtent::at(source, &(start..end), 0, source.len()),
                expected
            );
        }
        assert_eq!(
            after_line_deletion("use a::b; fn f() {}", "use a::b;"),
            " fn f() {}"
        );
    }

    #[test]
    fn neighboring_line_classification_retains_its_range() {
        assert_eq!(NeighborLine::at(" \nx\n", None), NeighborLine::Missing);
        assert_eq!(
            NeighborLine::at(" \nx\n", Some(0..2)),
            NeighborLine::Blank { range: 0..2 }
        );
        assert_eq!(
            NeighborLine::at(" \nx\n", Some(2..4)),
            NeighborLine::Occupied { range: 2..4 }
        );
        for (source, expected) in [
            ("mod a;\nuse a::b;\nmod b;\n", "mod a;\nmod b;\n"),
            ("mod a;\nuse a::b;", "mod a;\n"),
        ] {
            assert_eq!(after_line_deletion(source, "use a::b;"), expected);
        }
        assert_eq!(after_line_deletion("use a::b;\n\n", "use a::b;"), "");
        assert_eq!(after_line_deletion("\nuse a::b;", "use a::b;"), "");
        assert_eq!(
            after_line_deletion("use a::b;\ntrailing", "use a::b;"),
            "trailing"
        );
        assert_eq!(
            after_line_deletion("\nuse a::b;\ntrailing", "use a::b;"),
            "\ntrailing"
        );
    }
    #[test]
    fn recursive_identifier_presence_keeps_later_leaves_and_skips_doc_bodies() {
        for (text, expected) in [
            ("", IdentifierPresence::Absent),
            ("{ () [] } Name", IdentifierPresence::Present),
            ("{ (r#Name) }", IdentifierPresence::Present),
            ("#[doc = Name]", IdentifierPresence::Absent),
            ("#[doc = Name] unrelated Name", IdentifierPresence::Present),
            ("#[cfg(Name)]", IdentifierPresence::Present),
            ("NameSuffix", IdentifierPresence::Absent),
        ] {
            assert_eq!(tokens_name(text.parse().expect("tokens"), "Name"), expected);
        }
    }

    #[test]
    fn attribute_group_kind_requires_brackets_and_a_leading_doc_identifier() {
        for delimiter in [
            Delimiter::Bracket,
            Delimiter::Brace,
            Delimiter::Parenthesis,
            Delimiter::None,
        ] {
            for body in ["", "doc = Name", "cfg(Name)", "[doc]", "1", "::"] {
                let group = Group::new(delimiter, body.parse().expect("group body"));
                let expected = if delimiter == Delimiter::Bracket && body == "doc = Name" {
                    AttributeGroupKind::Documentation
                } else {
                    AttributeGroupKind::Traversable
                };
                assert_eq!(is_doc_attribute(&group), expected);
            }
        }
    }
}

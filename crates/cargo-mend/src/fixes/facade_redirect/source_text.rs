use std::collections::BTreeSet;
use std::ffi::OsStr;
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

    /// The byte offset of a span position. `LineColumn` counts characters, so
    /// the column is walked through the line rather than added as bytes.
    pub(in crate::fixes) fn offset(&self, position: LineColumn) -> usize {
        let line_start = self
            .line_starts
            .get(position.line.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        self.source[line_start..]
            .char_indices()
            .nth(position.column)
            .map_or(self.source.len(), |(index, _)| line_start + index)
    }

    /// The byte range of 1-based `line`, trailing newline included.
    pub(in crate::fixes) fn line_span(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line.checked_sub(1)?)?;
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
) -> (usize, usize) {
    (
        lines.offset(item_use.span().start()),
        lines.offset(item_use.semi_token.span.end()),
    )
}

/// A fix removing `range`, widened to whole lines when nothing else shares them.
pub(in crate::fixes) fn line_deletion(file: &Path, source: &str, range: Range<usize>) -> UseFix {
    let line_start = source[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[range.end..]
        .find('\n')
        .map_or(source.len(), |index| range.end + index + 1);
    let alone = source[line_start..range.start].trim().is_empty()
        && source[range.end..line_end].trim().is_empty();
    let (start, end) = if alone {
        with_neighboring_blank_line(source, line_start, line_end)
    } else {
        (range.start, range.end)
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
    let previous_is_blank = previous_line
        .clone()
        .is_some_and(|range| source[range].trim().is_empty());
    let next_line = source[line_end..]
        .find('\n')
        .map(|index| line_end..line_end + index + 1);
    let next_is_blank = next_line
        .clone()
        .is_some_and(|range| source[range].trim().is_empty());
    match (previous_line, next_line) {
        (None, Some(next)) if next_is_blank => (line_start, next.end),
        (Some(_), Some(next)) if previous_is_blank && next_is_blank => (line_start, next.end),
        (Some(previous), None) if previous_is_blank && line_end == source.len() => {
            (previous.start, line_end)
        },
        _ => (line_start, line_end),
    }
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
) -> bool {
    let mut blanked = text.to_string();
    for range in excluded {
        let blank = " ".repeat(range.len());
        blanked.replace_range(range, &blank);
    }
    blanked
        .parse::<TokenStream>()
        .map_or(true, |tokens| tokens_name(tokens, word))
}

/// Whether `tokens` hold `word` as an identifier or as an inline format
/// argument, skipping `#[doc = ...]` attribute bodies.
fn tokens_name(tokens: TokenStream, word: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident.unraw() == word,
        TokenTree::Group(group) => !is_doc_attribute(&group) && tokens_name(group.stream(), word),
        TokenTree::Literal(literal) => {
            let text = literal.to_string();
            text.contains(&format!("{{{word}}}")) || text.contains(&format!("{{{word}:"))
        },
        TokenTree::Punct(_) => false,
    })
}

/// Whether `group` is the bracketed body of a `#[doc = ...]` attribute, which
/// is how a doc comment lexes.
fn is_doc_attribute(group: &Group) -> bool {
    group.delimiter() == Delimiter::Bracket
        && matches!(
            group.stream().into_iter().next(),
            Some(TokenTree::Ident(ident)) if ident == "doc"
        )
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

    use proc_macro2::LineColumn;
    use syn::Item;
    use syn::parse_file;

    use super::SourceLines;
    use super::item_use_byte_range;
    use super::line_deletion;
    use super::word_occurs_outside;

    /// The text left after `line_deletion` removes the first occurrence of
    /// `target` from `source`.
    fn after_line_deletion(source: &str, target: &str) -> String {
        let start = source.find(target).expect("target in source");
        let fix = line_deletion(Path::new("lib.rs"), source, start..start + target.len());
        format!("{}{}", &source[..fix.start], &source[fix.end..])
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
        let offset = lines.offset(LineColumn {
            line:   2,
            column: 8,
        });
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
        let (start, end) = item_use_byte_range(&SourceLines::new(source), item_use);
        assert_eq!(
            &source[start..end],
            "#[doc = \"a; b\"]\npub use crate::x::Y;"
        );
    }

    #[test]
    fn a_blanked_range_hides_its_words() {
        let text = "use a::Name;\nfn f() {}\n";
        assert!(!word_occurs_outside(text, iter::once(0..12), "Name"));
        assert!(word_occurs_outside(text, iter::empty(), "Name"));
    }

    #[test]
    fn comments_and_string_literals_do_not_name_the_word() {
        let text = "//! Builds a [`Name`].\n/// A `Name`.\nfn f() -> &'static str {\n    // Name\n    \"Name\"\n}\n";
        assert!(!word_occurs_outside(text, iter::empty(), "Name"));
    }

    #[test]
    fn an_inline_format_argument_or_raw_identifier_names_the_word() {
        // Split so the test's own literal holds no format argument.
        let format_call = ["fn f() -> String { format!(\"{", "Name:?}\") }\n"].concat();
        assert!(word_occurs_outside(&format_call, iter::empty(), "Name"));
        assert!(word_occurs_outside(
            "fn f() { r#Name(); }\n",
            iter::empty(),
            "Name"
        ));
    }

    #[test]
    fn text_that_does_not_lex_names_the_word() {
        assert!(word_occurs_outside(
            "fn f() { \"open\n",
            iter::empty(),
            "Name"
        ));
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
}

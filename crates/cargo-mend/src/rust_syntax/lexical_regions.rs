use std::ops::Range;

use super::identifier_character;

/// What a [`LexicalRegion`] holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LexicalRegionKind {
    /// A line or block comment. Trivia — a backward walk passes over it.
    Comment,
    /// A string, raw string, or character literal. Its bytes are content, so a
    /// backward walk stops rather than reading them as whitespace.
    Literal,
}

/// One comment or literal, as a byte range of the source it was scanned from.
struct LexicalRegion {
    bytes: Range<usize>,
    kind:  LexicalRegionKind,
}

/// The state of a forward lex of one source file.
#[derive(Clone, Copy)]
enum RustLexicalState {
    Code,
    NormalString,
    RawString { hashes: usize },
    CharLiteral,
    LineComment,
    BlockComment { depth: usize },
}

impl RustLexicalState {
    /// The region kind this state accumulates, or `None` while lexing code.
    const fn region_kind(self) -> Option<LexicalRegionKind> {
        match self {
            Self::Code => None,
            Self::LineComment | Self::BlockComment { .. } => Some(LexicalRegionKind::Comment),
            Self::NormalString | Self::RawString { .. } | Self::CharLiteral => {
                Some(LexicalRegionKind::Literal)
            },
        }
    }
}

/// Where every comment and literal in one source file starts and ends.
///
/// Rust cannot be lexed backward: a `//` inside a string literal is spelled the
/// same as a comment opener, so whether a byte is trivia is only decidable by a
/// scan that starts at byte zero. Scanning once per file and keeping the
/// non-code spans lets [`Self::trivia_start_before`] answer from a binary
/// search plus a walk over the trivia run itself, instead of re-lexing the file
/// from the front for every offset a caller asks about.
pub(crate) struct LexicalRegions {
    /// Sorted by start offset, disjoint, and never adjacent — two comments with
    /// nothing but whitespace between them stay two regions, but a region never
    /// touches the next one, so at most one can contain a given offset.
    regions: Vec<LexicalRegion>,
}

impl From<&str> for LexicalRegions {
    fn from(source: &str) -> Self {
        let bytes = source.as_bytes();
        let end = source.len();
        let mut regions = Vec::new();
        let mut state = RustLexicalState::Code;
        let mut offset = 0;
        let mut region_start = 0;
        while offset < end {
            let previous = state;
            state = match previous {
                RustLexicalState::Code => {
                    region_start = offset;
                    let character = source[offset..].chars().next().unwrap_or_default();
                    if bytes.get(offset..offset + 2) == Some(b"//") {
                        offset += 2;
                        RustLexicalState::LineComment
                    } else if bytes.get(offset..offset + 2) == Some(b"/*") {
                        offset += 2;
                        RustLexicalState::BlockComment { depth: 1 }
                    } else if let Some((hashes, content_start)) =
                        raw_string_opening(bytes, offset, end)
                    {
                        offset = content_start;
                        RustLexicalState::RawString { hashes }
                    } else if bytes.get(offset) == Some(&b'"') {
                        offset += 1;
                        RustLexicalState::NormalString
                    } else if bytes.get(offset) == Some(&b'\'')
                        && starts_char_literal(source, offset, end)
                    {
                        offset += 1;
                        RustLexicalState::CharLiteral
                    } else {
                        offset += character.len_utf8();
                        RustLexicalState::Code
                    }
                },
                RustLexicalState::NormalString => match bytes.get(offset) {
                    Some(b'\\') => {
                        offset = (offset + 2).min(end);
                        RustLexicalState::NormalString
                    },
                    Some(b'"') => {
                        offset += 1;
                        RustLexicalState::Code
                    },
                    Some(_) => {
                        offset += 1;
                        RustLexicalState::NormalString
                    },
                    None => RustLexicalState::NormalString,
                },
                RustLexicalState::RawString { hashes } => {
                    if raw_string_closes(bytes, offset, end, hashes) {
                        offset += hashes + 1;
                        RustLexicalState::Code
                    } else {
                        offset += 1;
                        RustLexicalState::RawString { hashes }
                    }
                },
                RustLexicalState::CharLiteral => match bytes.get(offset) {
                    Some(b'\\') => {
                        offset = (offset + 2).min(end);
                        RustLexicalState::CharLiteral
                    },
                    Some(b'\'') => {
                        offset += 1;
                        RustLexicalState::Code
                    },
                    Some(_) => {
                        offset += 1;
                        RustLexicalState::CharLiteral
                    },
                    None => RustLexicalState::CharLiteral,
                },
                RustLexicalState::LineComment => {
                    if bytes.get(offset) == Some(&b'\n') {
                        offset += 1;
                        RustLexicalState::Code
                    } else {
                        offset += 1;
                        RustLexicalState::LineComment
                    }
                },
                RustLexicalState::BlockComment { depth } => {
                    if bytes.get(offset..offset + 2) == Some(b"/*") {
                        offset += 2;
                        RustLexicalState::BlockComment { depth: depth + 1 }
                    } else if bytes.get(offset..offset + 2) == Some(b"*/") {
                        offset += 2;
                        if depth == 1 {
                            RustLexicalState::Code
                        } else {
                            RustLexicalState::BlockComment { depth: depth - 1 }
                        }
                    } else {
                        offset += 1;
                        RustLexicalState::BlockComment { depth }
                    }
                },
            };
            if let Some(kind) = previous.region_kind()
                && matches!(state, RustLexicalState::Code)
            {
                regions.push(LexicalRegion {
                    bytes: region_start..offset,
                    kind,
                });
            }
        }
        // An unterminated comment or literal runs to the end of the file.
        if let Some(kind) = state.region_kind() {
            regions.push(LexicalRegion {
                bytes: region_start..end,
                kind,
            });
        }
        Self { regions }
    }
}

impl LexicalRegions {
    /// Where the run of whitespace and comments immediately before `end`
    /// begins, or `end` when the byte before `end` is not trivia.
    ///
    /// `source` must be the text these regions were scanned from. `end` falling
    /// inside a string or character literal answers `end`, because a literal's
    /// bytes are content rather than trivia however they are spelled.
    pub(crate) fn trivia_start_before(&self, source: &str, end: usize) -> usize {
        let mut offset = end;
        loop {
            if let Some(region) = self.region_ending_at_or_covering(offset) {
                match region.kind {
                    // The whole comment is trivia, so keep walking from its
                    // opener — whitespace may precede it in the same run.
                    LexicalRegionKind::Comment => {
                        offset = region.bytes.start;
                        continue;
                    },
                    LexicalRegionKind::Literal => return offset,
                }
            }
            let Some(character) = source
                .get(..offset)
                .and_then(|before| before.chars().next_back())
            else {
                return offset;
            };
            if !character.is_whitespace() {
                return offset;
            }
            offset -= character.len_utf8();
        }
    }

    /// The region containing `offset`, or the one whose last byte ends there.
    ///
    /// A region that merely starts at `offset` does not match: that offset is
    /// still a code position, the one a backward walk should stop at.
    fn region_ending_at_or_covering(&self, offset: usize) -> Option<&LexicalRegion> {
        let index = self
            .regions
            .partition_point(|region| region.bytes.end < offset);
        self.regions
            .get(index)
            .filter(|region| region.bytes.start < offset)
    }
}

fn raw_string_opening(bytes: &[u8], offset: usize, end: usize) -> Option<(usize, usize)> {
    if bytes.get(offset) != Some(&b'r') {
        return None;
    }
    let mut quote_offset = offset + 1;
    while quote_offset < end && bytes.get(quote_offset) == Some(&b'#') {
        quote_offset += 1;
    }
    (bytes.get(quote_offset) == Some(&b'"'))
        .then_some((quote_offset.saturating_sub(offset + 1), quote_offset + 1))
}

fn raw_string_closes(bytes: &[u8], offset: usize, end: usize, hashes: usize) -> bool {
    bytes.get(offset) == Some(&b'"')
        && offset + hashes < end
        && bytes
            .get(offset + 1..offset + hashes + 1)
            .is_some_and(|closing_hashes| closing_hashes.iter().all(|byte| *byte == b'#'))
}

fn starts_char_literal(source: &str, quote_offset: usize, end: usize) -> bool {
    let content_start = quote_offset + 1;
    let line_end = source[content_start..end]
        .find('\n')
        .map_or(end, |offset| content_start + offset);
    let Some(first_character) = source[content_start..line_end].chars().next() else {
        return false;
    };
    if identifier_character(first_character) {
        let identifier_end = source[content_start..line_end]
            .char_indices()
            .take_while(|(_, character)| identifier_character(*character))
            .last()
            .map_or(content_start, |(offset, character)| {
                content_start + offset + character.len_utf8()
            });
        return source
            .get(identifier_end..line_end)
            .is_some_and(|tail| tail.starts_with('\''));
    }

    let mut characters = source[content_start..line_end].chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == '\'' {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::LexicalRegions;

    fn trivia_start(source: &str, end: usize) -> usize {
        LexicalRegions::from(source).trivia_start_before(source, end)
    }

    #[test]
    fn plain_whitespace_run_reports_its_first_byte() {
        let source = "crate::  Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 7);
    }

    #[test]
    fn code_immediately_before_the_offset_reports_the_offset() {
        let source = "crate::Thing";
        let end = source.find("Thing").unwrap();
        assert_eq!(trivia_start(source, end), end);
    }

    #[test]
    fn comments_and_the_whitespace_around_them_are_one_run() {
        let source = "crate:: /* note */ // line\n Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 7);
    }

    #[test]
    fn nested_block_comments_close_at_the_outermost_terminator() {
        let source = "crate::/* outer /* inner */ still */Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 7);
    }

    #[test]
    fn a_comment_opener_inside_a_string_is_not_trivia() {
        let source = "let path = \"// not a comment\";Thing";
        let end = source.find("Thing").unwrap();
        assert_eq!(trivia_start(source, end), end);
    }

    #[test]
    fn an_offset_inside_a_string_literal_reports_itself() {
        let source = "let path = \"a  b\";";
        let end = source.find('b').unwrap();
        assert_eq!(trivia_start(source, end), end);
    }

    #[test]
    fn a_raw_string_hides_its_quotes_from_the_scan() {
        let source = "let path = r#\"a \" b\"#;  Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 22);
    }

    #[test]
    fn a_lifetime_is_not_read_as_a_character_literal() {
        let source = "fn f<'a>(x: &'a str) {}  Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 23);
    }

    #[test]
    fn an_escaped_quote_does_not_close_a_character_literal() {
        let source = "let quote = '\\'';  Thing";
        assert_eq!(trivia_start(source, source.find("Thing").unwrap()), 17);
    }

    #[test]
    fn an_unterminated_block_comment_runs_to_the_end_of_the_file() {
        let source = "crate:: /* never closed";
        assert_eq!(trivia_start(source, source.len()), 7);
    }
}

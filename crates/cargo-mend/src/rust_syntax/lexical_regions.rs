use std::num::NonZeroUsize;
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
///
/// Only an active comment or literal has a starting offset; code has no placeholder start.
/// A block comment has at least one open delimiter until its final level closes.
#[derive(Clone, Copy)]
enum RustLexicalState {
    Code,
    NormalString { start: usize },
    RawString { start: usize, hashes: usize },
    CharLiteral { start: usize },
    LineComment { start: usize },
    BlockComment { start: usize, depth: NonZeroUsize },
}

impl RustLexicalState {
    /// The active region's start and kind; code has no region to record.
    const fn region(self) -> Option<(usize, LexicalRegionKind)> {
        match self {
            Self::Code => None,
            Self::LineComment { start } | Self::BlockComment { start, .. } => {
                Some((start, LexicalRegionKind::Comment))
            },
            Self::NormalString { start }
            | Self::RawString { start, .. }
            | Self::CharLiteral { start } => Some((start, LexicalRegionKind::Literal)),
        }
    }
}

enum RustCodeOpening {
    LineComment,
    BlockComment,
    RawString {
        hashes:        usize,
        content_start: usize,
    },
    NormalString,
    CharLiteral,
    CodeCharacter {
        utf8_width: usize,
    },
}

impl RustCodeOpening {
    fn at(source: &str, offset: usize, end: usize) -> Self {
        let bytes = source.as_bytes();
        if bytes.get(offset..offset + 2) == Some(b"//") {
            return Self::LineComment;
        }
        if bytes.get(offset..offset + 2) == Some(b"/*") {
            return Self::BlockComment;
        }
        if let Some((hashes, content_start)) = raw_string_opening(bytes, offset, end) {
            return Self::RawString {
                hashes,
                content_start,
            };
        }
        if bytes.get(offset) == Some(&b'"') {
            return Self::NormalString;
        }
        if bytes.get(offset) == Some(&b'\'') {
            match apostrophe_role(source, offset, end) {
                ApostropheRole::CharacterLiteral => return Self::CharLiteral,
                ApostropheRole::LifetimeOrUnclosed => {},
            }
        }
        Self::CodeCharacter {
            utf8_width: source[offset..]
                .chars()
                .next()
                .unwrap_or_default()
                .len_utf8(),
        }
    }
}

impl RustCodeOpening {
    const fn advance(self, offset: &mut usize) -> RustLexicalState {
        let start = *offset;
        match self {
            Self::LineComment => {
                *offset += 2;
                RustLexicalState::LineComment { start }
            },
            Self::BlockComment => {
                *offset += 2;
                RustLexicalState::BlockComment {
                    start,
                    depth: NonZeroUsize::MIN,
                }
            },
            Self::RawString {
                hashes,
                content_start,
            } => {
                *offset = content_start;
                RustLexicalState::RawString { start, hashes }
            },
            Self::NormalString => {
                *offset += 1;
                RustLexicalState::NormalString { start }
            },
            Self::CharLiteral => {
                *offset += 1;
                RustLexicalState::CharLiteral { start }
            },
            Self::CodeCharacter { utf8_width } => {
                *offset += utf8_width;
                RustLexicalState::Code
            },
        }
    }
}

enum BlockCommentToken {
    Open,
    Close,
    Content,
}

impl From<&[u8]> for BlockCommentToken {
    fn from(bytes: &[u8]) -> Self {
        match bytes {
            [b'/', b'*', ..] => Self::Open,
            [b'*', b'/', ..] => Self::Close,
            _ => Self::Content,
        }
    }
}

enum CharacterLiteralToken {
    Escape,
    Close,
    Content,
}

impl From<char> for CharacterLiteralToken {
    fn from(character: char) -> Self {
        match character {
            '\\' => Self::Escape,
            '\'' => Self::Close,
            _ => Self::Content,
        }
    }
}

enum ApostropheRole {
    CharacterLiteral,
    LifetimeOrUnclosed,
}

/// Where every comment and literal in one source file starts and ends.
///
/// Rust cannot be lexed backward: a `//` inside a string literal is spelled the
/// same as a comment opener, so whether a byte is trivia is only decidable by a
/// scan that starts at byte zero. Scanning once per file and keeping the
/// non-code spans lets [`SourceEndpoint::trivia_start_before`] answer from a binary
/// search plus a walk over the trivia run itself, instead of re-lexing the file
/// from the front for every offset a caller asks about.
/// The original source is owned with the index, so callers cannot supply different text.
pub(crate) struct LexicalRegions {
    source:  String,
    /// Sorted by start offset, disjoint, and never adjacent — two comments with
    /// nothing but whitespace between them stay two regions, but a region never
    /// touches the next one, so at most one can contain a given offset.
    regions: Vec<LexicalRegion>,
}

impl From<String> for LexicalRegions {
    fn from(source: String) -> Self {
        let bytes = source.as_bytes();
        let end = source.len();
        let mut regions = Vec::new();
        let mut state = RustLexicalState::Code;
        let mut offset = 0;
        while offset < end {
            let previous = state;
            state = match previous {
                RustLexicalState::Code => {
                    RustCodeOpening::at(&source, offset, end).advance(&mut offset)
                },
                RustLexicalState::NormalString { start } => match bytes.get(offset) {
                    Some(b'\\') => {
                        offset = (offset + 2).min(end);
                        RustLexicalState::NormalString { start }
                    },
                    Some(b'"') => {
                        offset += 1;
                        RustLexicalState::Code
                    },
                    Some(_) => {
                        offset += 1;
                        RustLexicalState::NormalString { start }
                    },
                    None => RustLexicalState::NormalString { start },
                },
                RustLexicalState::RawString { start, hashes } => {
                    if raw_string_closes(bytes, offset, end, hashes) {
                        offset += hashes + 1;
                        RustLexicalState::Code
                    } else {
                        offset += 1;
                        RustLexicalState::RawString { start, hashes }
                    }
                },
                RustLexicalState::CharLiteral { start } => match bytes.get(offset) {
                    Some(b'\\') => {
                        offset = (offset + 2).min(end);
                        RustLexicalState::CharLiteral { start }
                    },
                    Some(b'\'') => {
                        offset += 1;
                        RustLexicalState::Code
                    },
                    Some(_) => {
                        offset += 1;
                        RustLexicalState::CharLiteral { start }
                    },
                    None => RustLexicalState::CharLiteral { start },
                },
                RustLexicalState::LineComment { start } => {
                    if bytes.get(offset) == Some(&b'\n') {
                        offset += 1;
                        RustLexicalState::Code
                    } else {
                        offset += 1;
                        RustLexicalState::LineComment { start }
                    }
                },
                RustLexicalState::BlockComment { start, depth } => {
                    match BlockCommentToken::from(&bytes[offset..]) {
                        BlockCommentToken::Open => {
                            offset += 2;
                            RustLexicalState::BlockComment {
                                start,
                                depth: depth.saturating_add(1),
                            }
                        },
                        BlockCommentToken::Close => {
                            offset += 2;
                            NonZeroUsize::new(depth.get() - 1)
                                .map_or(RustLexicalState::Code, |depth| {
                                    RustLexicalState::BlockComment { start, depth }
                                })
                        },
                        BlockCommentToken::Content => {
                            offset += 1;
                            RustLexicalState::BlockComment { start, depth }
                        },
                    }
                },
            };
            if let Some((start, kind)) = previous.region()
                && matches!(state, RustLexicalState::Code)
            {
                regions.push(LexicalRegion {
                    bytes: start..offset,
                    kind,
                });
            }
        }
        // `regions.push` records an unterminated comment or literal through the end of the file.
        if let Some((start, kind)) = state.region() {
            regions.push(LexicalRegion {
                bytes: start..end,
                kind,
            });
        }
        Self { source, regions }
    }
}

/// A UTF-8 boundary in the exact source whose comments and literals were indexed.
///
/// Raw endpoints may exceed the source or split a character. Pairing an index with
/// different text can also misclassify trivia. Only the owning index can construct
/// an endpoint; backward operations retain that owner and preserve valid boundaries.
#[derive(Clone, Copy)]
pub(crate) struct SourceEndpoint<'source> {
    owner:  &'source LexicalRegions,
    offset: usize,
}

impl<'source> SourceEndpoint<'source> {
    pub(crate) const fn offset(self) -> usize { self.offset }

    pub(crate) fn prefix(self) -> &'source str { &self.owner.source[..self.offset] }

    pub(crate) fn strip_suffix(self, suffix: &str) -> Option<Self> {
        self.prefix().strip_suffix(suffix).map(|prefix| Self {
            offset: prefix.len(),
            ..self
        })
    }

    pub(crate) fn trim_end_matches(self, predicate: impl FnMut(char) -> bool) -> Self {
        Self {
            offset: self.prefix().trim_end_matches(predicate).len(),
            ..self
        }
    }

    /// Where whitespace and comments immediately before this endpoint begin.
    /// An endpoint inside a literal stays put: those bytes are content, not trivia.
    pub(crate) fn trivia_start_before(self) -> Self {
        let mut offset = self.offset;
        loop {
            if let Some(region) = self.owner.region_ending_at_or_covering(offset) {
                match region.kind {
                    // Comments are entirely trivia; whitespace may precede the opener.
                    LexicalRegionKind::Comment => {
                        offset = region.bytes.start;
                        continue;
                    },
                    LexicalRegionKind::Literal => return Self { offset, ..self },
                }
            }
            let Some(character) = self.owner.source[..offset].chars().next_back() else {
                return Self { offset, ..self };
            };
            if !character.is_whitespace() {
                return Self { offset, ..self };
            }
            offset -= character.len_utf8();
        }
    }
}

impl LexicalRegions {
    pub(crate) fn source(&self) -> &str { &self.source }

    /// Reject out-of-source offsets and bytes inside a UTF-8 character once, at entry.
    pub(crate) fn endpoint(&self, offset: usize) -> Option<SourceEndpoint<'_>> {
        self.source
            .is_char_boundary(offset)
            .then_some(SourceEndpoint {
                owner: self,
                offset,
            })
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

fn apostrophe_role(source: &str, quote_offset: usize, end: usize) -> ApostropheRole {
    let content_start = quote_offset + 1;
    let line_end = source[content_start..end]
        .find('\n')
        .map_or(end, |offset| content_start + offset);
    let Some(first_character) = source[content_start..line_end].chars().next() else {
        return ApostropheRole::LifetimeOrUnclosed;
    };
    if identifier_character(first_character) {
        let identifier_end = source[content_start..line_end]
            .char_indices()
            .take_while(|(_, character)| identifier_character(*character))
            .last()
            .map_or(content_start, |(offset, character)| {
                content_start + offset + character.len_utf8()
            });
        return if source
            .get(identifier_end..line_end)
            .is_some_and(|tail| tail.starts_with('\''))
        {
            ApostropheRole::CharacterLiteral
        } else {
            ApostropheRole::LifetimeOrUnclosed
        };
    }

    let mut characters = source[content_start..line_end].chars();
    while let Some(character) = characters.next() {
        match CharacterLiteralToken::from(character) {
            CharacterLiteralToken::Escape => {
                characters.next();
            },
            CharacterLiteralToken::Close => return ApostropheRole::CharacterLiteral,
            CharacterLiteralToken::Content => {},
        }
    }
    ApostropheRole::LifetimeOrUnclosed
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::LexicalRegions;

    fn trivia_start(source: &str, end: usize) -> usize {
        LexicalRegions::from(source.to_owned())
            .endpoint(end)
            .unwrap()
            .trivia_start_before()
            .offset()
    }

    #[test]
    fn endpoints_reject_out_of_source_and_split_character_offsets() {
        let regions = LexicalRegions::from("é /* comment */  ".to_owned());
        assert!(regions.endpoint(1).is_none());
        assert!(regions.endpoint(regions.source().len() + 1).is_none());
        assert!(regions.endpoint(usize::MAX).is_none());
        assert_eq!(
            regions.endpoint(0).unwrap().trivia_start_before().offset(),
            0
        );
        assert_eq!(
            regions
                .endpoint(regions.source().len())
                .unwrap()
                .trivia_start_before()
                .prefix(),
            "é",
        );
        let empty = LexicalRegions::from(String::new());
        assert_eq!(empty.endpoint(0).unwrap().trivia_start_before().offset(), 0);
        assert!(empty.endpoint(1).is_none());
    }

    #[test]
    fn endpoints_keep_their_owner_and_the_original_source_allocation() {
        let source = "é /*tail*/".to_owned();
        let allocation = source.as_ptr();
        let comments = LexicalRegions::from(source);
        let literal = LexicalRegions::from("é \"tail  \"".to_owned());
        assert_eq!(comments.source().as_ptr(), allocation);
        assert_eq!(comments.source().len(), literal.source().len());
        let comment_end = comments.endpoint(comments.source().len()).unwrap();
        let literal_end = literal.endpoint(literal.source().len()).unwrap();
        assert_eq!(comment_end.trivia_start_before().prefix(), "é");
        assert_eq!(literal_end.trivia_start_before().prefix(), literal.source());
        assert!(std::ptr::eq(
            comment_end.trivia_start_before().owner,
            &raw const comments
        ));
        assert!(std::ptr::eq(
            literal_end.trivia_start_before().owner,
            &raw const literal
        ));
    }

    #[test]
    fn each_region_keeps_its_own_start_across_code_and_unterminated_content() {
        let source = concat!(
            "head \"one\" tail /* outer /* inner */ end */ next // line\n",
            " end r#\"open",
        );
        let regions = LexicalRegions::from(source.to_owned());
        let expected = [
            "\"one\"",
            "/* outer /* inner */ end */",
            "// line\n",
            "r#\"open",
        ]
        .map(|text| {
            let start = source.find(text).unwrap();
            start..start + text.len()
        });
        assert_eq!(
            regions
                .regions
                .iter()
                .map(|region| region.bytes.clone())
                .collect::<Vec<_>>(),
            expected,
        );
        for (region, expected) in regions.regions.iter().zip(expected) {
            assert_eq!(&source[region.bytes.clone()], &source[expected]);
        }
    }

    #[test]
    fn closing_nested_comments_keeps_the_outer_region_live_until_its_last_level() {
        for comment in [
            "/* closed */",
            "/* outer /* inner */ end */",
            "/* outer /* inner */ unfinished",
            "/* outer /* middle /* inner */ end */ final */",
        ] {
            let source = format!("é {comment}");
            let regions = LexicalRegions::from(source.clone());
            assert_eq!(regions.regions.len(), 1, "{comment}");
            assert_eq!(regions.regions[0].bytes, "é ".len()..source.len());
            assert_eq!(
                regions
                    .endpoint(source.len())
                    .unwrap()
                    .trivia_start_before()
                    .offset(),
                "é".len()
            );
        }
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
    #[test]
    fn lexical_regions_preserve_delimiters_utf8_and_unterminated_content() {
        for (source, expected) in [
            ("é/* outer /* nested */ end */", Some((2, "comment"))),
            (r###"r##"/* " still raw"##"###, Some((0, "literal"))),
            ("'é'", Some((0, "literal"))),
            (r"'\''", Some((0, "literal"))),
            ("'!x'", Some((0, "literal"))),
            ("'life ", None),
            ("'", None),
            ("'!", None),
            ("\"//", Some((0, "literal"))),
            ("r#\"/*", Some((0, "literal"))),
            ("//tail", Some((0, "comment"))),
            ("/*tail", Some((0, "comment"))),
        ] {
            let regions = LexicalRegions::from(source.to_owned())
                .regions
                .into_iter()
                .map(|region| {
                    let kind = match region.kind {
                        super::LexicalRegionKind::Comment => "comment",
                        super::LexicalRegionKind::Literal => "literal",
                    };
                    (region.bytes, kind)
                })
                .collect::<Vec<_>>();
            let expected = expected
                .into_iter()
                .map(|(start, kind)| (start..source.len(), kind))
                .collect::<Vec<_>>();
            assert_eq!(regions, expected, "{source:?}");
        }
    }
}

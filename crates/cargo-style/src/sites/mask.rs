//! Mask comments and literals. Preserve byte positions and LF characters.

use std::ops::Range;

use super::constants::DISPLAY_LIMIT;
use super::constants::ESCAPED_CHAR_LIMIT;
use super::frontmatter;
use crate::source_lines;
use crate::source_sites;

pub(super) struct MaskedSource {
    bytes:    String,
    lines:    LineText,
    literals: Vec<Literal>,
}

enum LineText {
    ByteAligned,
    CodePointAligned(String),
}

impl MaskedSource {
    pub(super) fn text(&self) -> &str { &self.bytes }

    pub(super) fn literals(&self) -> &[Literal] { &self.literals }

    pub(super) fn lines(&self) -> impl Iterator<Item = &str> {
        let text = match &self.lines {
            LineText::ByteAligned => self.text(),
            LineText::CodePointAligned(text) => text,
        };
        lines(text)
    }

    fn copy(&mut self, text: &str) {
        self.bytes.push_str(text);
        if let LineText::CodePointAligned(lines) = &mut self.lines {
            lines.push_str(text);
        }
    }

    fn blank(&mut self, text: &str) {
        if matches!(self.lines, LineText::ByteAligned) && !text.is_ascii() {
            // Python replaces each code point with one space.
            self.lines = LineText::CodePointAligned(self.bytes.clone());
        }
        if let LineText::CodePointAligned(lines) = &mut self.lines {
            lines.extend(
                text.chars()
                    .map(|character| if character == '\n' { '\n' } else { ' ' }),
            );
        }
        self.bytes.extend(
            text.bytes()
                .map(|byte| if byte == b'\n' { '\n' } else { ' ' }),
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Literal {
    pub(super) line:   usize,
    pub(super) column: usize,
    pub(super) text:   String,
    pub(super) kind:   LiteralKind,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum LiteralKind {
    String,
    Char,
}

struct LiteralSpan {
    end:      usize,
    contents: Range<usize>,
    kind:     LiteralKind,
}

struct LiteralPosition {
    offset: usize,
    line:   usize,
    column: usize,
}

impl LiteralPosition {
    fn record(&mut self, text: &str, start: usize, span: LiteralSpan) -> Literal {
        let quote = start + span.contents.start - 1;
        for character in text[self.offset..quote].chars() {
            if character == '\n' {
                self.line += 1;
                self.column = 0;
            } else {
                self.column += 1;
            }
        }
        self.offset = quote;
        Literal {
            line:   self.line,
            column: self.column,
            text:   text[start + span.contents.start..start + span.contents.end].to_owned(),
            kind:   span.kind,
        }
    }
}

enum MaskSpan {
    Keep,
    Comment(usize),
    Literal(LiteralSpan),
}

pub(super) fn mask_source(text: &str) -> MaskedSource {
    let bytes = text.as_bytes();
    let mut masked = MaskedSource {
        bytes:    String::with_capacity(text.len()),
        lines:    LineText::ByteAligned,
        literals: Vec::new(),
    };
    let mut position = LiteralPosition {
        offset: 0,
        line:   1,
        column: 0,
    };
    let mut copied = 0;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let end = match mask_span(text, cursor) {
            MaskSpan::Keep => {
                cursor += 1;
                continue;
            },
            MaskSpan::Comment(end) => end,
            MaskSpan::Literal(span) => {
                let end = span.end;
                masked.literals.push(position.record(text, cursor, span));
                end
            },
        };
        masked.copy(&text[copied..cursor]);
        masked.blank(&text[cursor..cursor + end]);
        cursor += end;
        copied = cursor;
    }
    masked.copy(&text[copied..]);
    masked
}

fn mask_span(text: &str, cursor: usize) -> MaskSpan {
    let bytes = text.as_bytes();
    let tail = &bytes[cursor..];
    match tail {
        [b'/', b'/', ..] => MaskSpan::Comment(
            tail.iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(tail.len()),
        ),
        [b'/', b'*', ..] => MaskSpan::Comment(block_comment_end(tail)),
        [b'"', ..] => MaskSpan::Literal(string_span(tail)),
        [b'\'', ..] => {
            let end = char_end(&text[cursor..]);
            if end == 0 {
                MaskSpan::Keep
            } else {
                MaskSpan::Literal(LiteralSpan {
                    end,
                    contents: 1..end - 1,
                    kind: LiteralKind::Char,
                })
            }
        },
        [b'b' | b'r', ..] if raw_boundary(bytes, cursor) => raw_span(&text[cursor..]),
        _ => MaskSpan::Keep,
    }
}

pub(super) fn display_line(text: &str, line: usize) -> String {
    line.checked_sub(1)
        .and_then(|index| lines(text).nth(index))
        .map_or_else(String::new, |line| {
            source_sites::trim(line)
                .chars()
                .take(DISPLAY_LIMIT)
                .collect()
        })
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    source_lines::split_lines(text).map(frontmatter::without_line_ending)
}

fn block_comment_end(tail: &[u8]) -> usize {
    let mut depth = 1;
    let mut cursor = 2;
    while let Some(pair) = tail.get(cursor..cursor + 2) {
        match pair {
            b"/*" => {
                depth += 1;
                cursor += 2;
            },
            b"*/" => {
                depth -= 1;
                cursor += 2;
                if depth == 0 {
                    return cursor;
                }
            },
            _ => cursor += 1,
        }
    }
    tail.len()
}

const fn raw_boundary(bytes: &[u8], cursor: usize) -> bool {
    cursor == 0 || !(bytes[cursor - 1].is_ascii_alphanumeric() || bytes[cursor - 1] == b'_')
}

fn raw_span(text: &str) -> MaskSpan {
    let tail = text.as_bytes();
    let rest = tail.strip_prefix(b"br").or_else(|| tail.strip_prefix(b"r"));
    let Some(rest) = rest else {
        return MaskSpan::Keep;
    };
    let hashes = rest.iter().take_while(|&&byte| byte == b'#').count();
    if rest.get(hashes) != Some(&b'"') {
        return MaskSpan::Keep;
    }
    let start = tail.len() - rest.len() + hashes + 1;
    let closer = hashes + 1;
    let close = tail[start..]
        .windows(closer)
        .position(|bytes| bytes[0] == b'"' && bytes[1..] == rest[..hashes]);
    let (end, content_end) = close.map_or_else(
        || {
            // Python subtracts the closer length even when it cannot find the closer.
            let content_end = text[start..]
                .char_indices()
                .nth_back(hashes)
                .map_or(start, |(offset, _)| start + offset);
            (tail.len(), content_end)
        },
        |offset| (start + offset + closer, start + offset),
    );
    MaskSpan::Literal(LiteralSpan {
        end,
        contents: start..content_end,
        kind: LiteralKind::String,
    })
}

fn string_span(tail: &[u8]) -> LiteralSpan {
    let mut cursor = 1;
    while let Some(&byte) = tail.get(cursor) {
        match byte {
            b'"' => {
                return LiteralSpan {
                    end:      cursor + 1,
                    contents: 1..cursor,
                    kind:     LiteralKind::String,
                };
            },
            b'\\' => cursor += 2,
            _ => cursor += 1,
        }
    }
    LiteralSpan {
        end:      tail.len(),
        contents: 1..tail.len(),
        kind:     LiteralKind::String,
    }
}

fn char_end(tail: &str) -> usize {
    let mut characters = tail[1..].char_indices();
    if tail.as_bytes().get(1) == Some(&b'\\') {
        characters
            .take(ESCAPED_CHAR_LIMIT)
            .find_map(|(offset, character)| (character == '\'').then_some(offset + 2))
            .unwrap_or(0)
    } else {
        characters
            .nth(1)
            .filter(|&(_, character)| character == '\'')
            .map_or(0, |(offset, _)| offset + 2)
    }
}

#[cfg(test)]
mod tests {
    use LiteralKind::Char;
    use LiteralKind::String;

    use super::LiteralKind;
    use super::display_line;
    use super::mask_source;

    #[test]
    fn masks_literals_and_comments_at_their_byte_positions() {
        let cases = [
            ("a/* /* */\r\n */z", "a         \n   z"),
            ("a//é\r\nz", "a     \nz"),
            ("a/* unclosed\nz", "a           \n "),
            ("a\"unclosed\\", "a          "),
            ("\"é\"z", "    z"),
            ("b\"x\" br##\"a\"#b\"##z", "b                z"),
            ("r#\"a\"z", "      "),
            ("'x' 'a '\\n' z", "    'a      z"),
            ("'\\123456789012' z", "'\\123456789012' z"),
        ];
        for (source, expected) in cases {
            assert_eq!(mask_source(source).text(), expected, "{source:?}");
        }
    }

    #[test]
    fn displays_the_original_line_with_python_whitespace() {
        let text = "  first\r\n\u{1c}  \"literal\" \u{a0}\r\n";
        assert_eq!(display_line(text, 3), "\"literal\"");
        assert_eq!(display_line(text, 4), "");
    }

    #[test]
    fn retains_python_spacing_in_masked_lines() {
        let masked = mask_source("a/*é*/b");
        assert_eq!(masked.text(), "a      b");
        assert_eq!(masked.lines().next(), Some("a     b"));
    }

    #[test]
    fn records_literal_contents_and_opening_quote_positions() {
        let cases = [
            ("\"hello\"", vec![(1, 0, "hello", String)]),
            ("b\"a\\\"b\"", vec![(1, 1, "a\\\"b", String)]),
            ("c\"plain\"", vec![(1, 1, "plain", String)]),
            (
                "cr#\"left\"middle\"#",
                vec![(1, 3, "left", String), (1, 15, "#", String)],
            ),
            ("'é' b'\\n'", vec![(1, 0, "é", Char), (1, 5, "\\n", Char)]),
            (
                "r\"raw\" br##\"a\"#b\"##",
                vec![(1, 1, "raw", String), (1, 11, "a\"#b", String)],
            ),
            (
                "/* \"skip\" */é\"x\nz\"\r\nb'q'",
                vec![(1, 13, "x\nz", String), (3, 1, "q", Char)],
            ),
            (
                "// \"skip\"\n\"unclosed\\",
                vec![(2, 0, "unclosed\\", String)],
            ),
            ("r#\"abcéλ", vec![(1, 2, "abc", String)]),
            ("'life \"\"", vec![(1, 6, "", String)]),
        ];
        for (source, expected) in cases {
            let masked = mask_source(source);
            let actual = masked
                .literals()
                .iter()
                .map(|literal| {
                    (
                        literal.line,
                        literal.column,
                        literal.text.as_str(),
                        &literal.kind,
                    )
                })
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .map(|(line, column, text, kind)| (*line, *column, *text, kind))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{source:?}");
        }
    }
}

//! Mask comments and literals. Preserve byte positions and LF characters.

use super::constants::DISPLAY_LIMIT;
use super::constants::ESCAPED_CHAR_LIMIT;
use super::frontmatter;
use crate::source_lines;
use crate::tokenizer;

pub(super) struct MaskedSource {
    bytes: String,
    lines: LineText,
}

enum LineText {
    ByteAligned,
    CodePointAligned(String),
}

impl MaskedSource {
    pub(super) fn text(&self) -> &str { &self.bytes }

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

pub(super) fn mask_source(text: &str) -> MaskedSource {
    let bytes = text.as_bytes();
    let mut masked = MaskedSource {
        bytes: String::with_capacity(text.len()),
        lines: LineText::ByteAligned,
    };
    let mut copied = 0;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let tail = &bytes[cursor..];
        let end = match tail {
            [b'/', b'/', ..] => tail
                .iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(tail.len()),
            [b'/', b'*', ..] => block_comment_end(tail),
            [b'"', ..] => string_end(tail),
            [b'\'', ..] => char_end(&text[cursor..]),
            [b'b' | b'r', ..] if raw_boundary(bytes, cursor) => raw_end(tail),
            _ => 0,
        };
        if end == 0 {
            cursor += 1;
            continue;
        }
        masked.copy(&text[copied..cursor]);
        masked.blank(&text[cursor..cursor + end]);
        cursor += end;
        copied = cursor;
    }
    masked.copy(&text[copied..]);
    masked
}

pub(super) fn display_line(text: &str, line: usize) -> String {
    line.checked_sub(1)
        .and_then(|index| lines(text).nth(index))
        .map_or_else(String::new, |line| {
            line.trim_matches(tokenizer::python_space)
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

fn raw_end(tail: &[u8]) -> usize {
    let rest = tail.strip_prefix(b"br").or_else(|| tail.strip_prefix(b"r"));
    let Some(rest) = rest else { return 0 };
    let hashes = rest.iter().take_while(|&&byte| byte == b'#').count();
    if rest.get(hashes) != Some(&b'"') {
        return 0;
    }
    let start = tail.len() - rest.len() + hashes + 1;
    let closer = hashes + 1;
    tail[start..]
        .windows(closer)
        .position(|bytes| bytes[0] == b'"' && bytes[1..] == rest[..hashes])
        .map_or(tail.len(), |offset| start + offset + closer)
}

fn string_end(tail: &[u8]) -> usize {
    let mut cursor = 1;
    while let Some(&byte) = tail.get(cursor) {
        match byte {
            b'"' => return cursor + 1,
            b'\\' => cursor += 2,
            _ => cursor += 1,
        }
    }
    tail.len()
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
}

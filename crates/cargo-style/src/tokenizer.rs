//! A one-to-one port of `tokenized_text` in `scripts/style/style_footprint.py`.
//!
//! The Python reader drops comments, keeps literals opaque, and makes one `unknown` token for a
//! character it cannot read. Each match below follows one Python regular expression in the same
//! order, so both readers give the same tokens. Offsets count code points, as Python string
//! offsets do.

use serde::Serialize;

use crate::constants::PUNCTUATION_PAIRS;
use crate::constants::PUNCTUATION_SINGLES;
use crate::constants::UNTERMINATED_COMMENT;
use crate::unicode_tables::DECIMAL;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

/// The Python token kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TokenKind {
    Identifier,
    Literal,
    Punctuation,
    Unknown,
}

/// One code token, with the line and the code-point offset where it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Token {
    pub(crate) value: String,
    pub(crate) kind:  TokenKind,
    pub(crate) line:  usize,
    pub(crate) start: usize,
}

/// One complete comment and its position among the code tokens.
pub(crate) struct CommentSpan {
    pub(crate) start:          usize,
    pub(crate) end:            usize,
    pub(crate) token_position: usize,
}

/// The code tokens and complete comments from one lexical reading.
#[derive(Default)]
pub(crate) struct TokenReading {
    pub(crate) tokens:   Vec<Token>,
    pub(crate) comments: Vec<CommentSpan>,
}

/// What the reader does at the cursor.
#[derive(Clone, Copy)]
enum Step {
    /// Move the cursor to end with no token: space or a comment.
    Skip { end: usize },
    /// Record a complete comment and move the cursor to its end.
    Comment { end: usize },
    /// Push the text from the cursor to end as one token.
    Token { end: usize, kind: TokenKind },
    /// A block comment has no end: push one unknown token at the end of the text.
    UnterminatedComment,
}

/// A lexical step that normalization does not change.
struct UnchangedStep {
    start: usize,
    step:  Step,
}

/// Reads the tokens of one source text and records the line and start of each.
struct Tokenizer<'a> {
    text:     &'a [char],
    reused:   &'a [UnchangedStep],
    cursor:   usize,
    // The offset where `line` was last brought up to date.
    counted:  usize,
    line:     usize,
    tokens:   Vec<Token>,
    comments: Vec<CommentSpan>,
}

impl<'a> Tokenizer<'a> {
    const fn new(text: &'a [char]) -> Self {
        Self {
            text,
            reused: &[],
            cursor: 0,
            counted: 0,
            line: 1,
            tokens: Vec::new(),
            comments: Vec::new(),
        }
    }

    fn run(self) -> Vec<Token> { self.read().tokens }

    fn read(mut self) -> TokenReading {
        while self.cursor < self.text.len() {
            match self.next_step() {
                Step::Skip { end } => self.cursor = end,
                Step::Comment { end } => {
                    self.comments.push(CommentSpan {
                        start: self.cursor,
                        end,
                        token_position: self.tokens.len(),
                    });
                    self.cursor = end;
                },
                Step::Token { end, kind } => {
                    let value = self.text[self.cursor..end].iter().collect();
                    self.push(value, kind);
                    self.cursor = end;
                },
                Step::UnterminatedComment => {
                    self.cursor = self.text.len();
                    self.push(UNTERMINATED_COMMENT.to_owned(), TokenKind::Unknown);
                },
            }
        }
        TokenReading {
            tokens:   self.tokens,
            comments: self.comments,
        }
    }

    /// The Python reader tries each pattern in this order; a character no pattern reads is an
    /// unknown token.
    fn next_step(&self) -> Step {
        let index = self
            .reused
            .partition_point(|reading| reading.start < self.cursor);
        if let Some(reading) = self.reused.get(index)
            && reading.start == self.cursor
        {
            return reading.step;
        }
        self.space()
            .or_else(|| self.line_comment())
            .or_else(|| self.block_comment())
            .or_else(|| self.raw_string())
            .or_else(|| self.quoted_literal())
            .or_else(|| self.number())
            .or_else(|| self.identifier())
            .or_else(|| self.punctuation())
            .unwrap_or(Step::Token {
                end:  self.cursor + 1,
                kind: TokenKind::Unknown,
            })
    }

    /// Push a token that starts at the cursor.
    fn push(&mut self, value: String, kind: TokenKind) {
        self.line += self.text[self.counted..self.cursor]
            .iter()
            .filter(|&&character| character == '\n')
            .count();
        self.counted = self.cursor;
        self.tokens.push(Token {
            value,
            kind,
            line: self.line,
            start: self.cursor,
        });
    }

    /// Python `str.isspace`.
    fn space(&self) -> Option<Step> {
        in_table(SPACE, self.text[self.cursor]).then_some(Step::Skip {
            end: self.cursor + 1,
        })
    }

    /// A line comment ends before its newline.
    fn line_comment(&self) -> Option<Step> {
        self.starts_with(self.cursor, &['/', '/'])
            .then(|| Step::Comment {
                end: self.find(self.cursor, &['\n']).unwrap_or(self.text.len()),
            })
    }

    /// Block comments nest.
    fn block_comment(&self) -> Option<Step> {
        if !self.starts_with(self.cursor, &['/', '*']) {
            return None;
        }
        let mut depth = 1_usize;
        let mut index = self.cursor + 2;
        while index < self.text.len() && depth > 0 {
            match self.text.get(index..index + 2) {
                Some(['/', '*']) => {
                    depth += 1;
                    index += 2;
                },
                Some(['*', '/']) => {
                    depth -= 1;
                    index += 2;
                },
                _ => index += 1,
            }
        }
        Some(if depth > 0 {
            Step::UnterminatedComment
        } else {
            Step::Comment { end: index }
        })
    }

    /// Python `RAW_STRING`: `(?:br|cr|r)(#*)"`. The string ends at a quote and the same hashes;
    /// with no end, it is one unknown token to the end of the text.
    fn raw_string(&self) -> Option<Step> {
        let hashes_start = match self.text.get(self.cursor..self.cursor + 2) {
            Some(['b' | 'c', 'r']) => self.cursor + 2,
            _ if self.text[self.cursor] == 'r' => self.cursor + 1,
            _ => return None,
        };
        let quote = self.run_end(hashes_start, |character| character == '#');
        if self.text.get(quote) != Some(&'"') {
            return None;
        }
        let terminator: Vec<char> = std::iter::once('"')
            .chain(std::iter::repeat_n('#', quote - hashes_start))
            .collect();
        Some(self.find(quote + 1, &terminator).map_or(
            Step::Token {
                end:  self.text.len(),
                kind: TokenKind::Unknown,
            },
            |close| Step::Token {
                end:  close + terminator.len(),
                kind: TokenKind::Literal,
            },
        ))
    }

    /// Python `QUOTED_LITERAL`: a string, then a character literal.
    fn quoted_literal(&self) -> Option<Step> {
        self.string_end()
            .or_else(|| self.character_end())
            .map(|end| Step::Token {
                end,
                kind: TokenKind::Literal,
            })
    }

    /// `(?:b|c)?"(?:\\.|[^"\\])*"`, with `.` matching a newline.
    fn string_end(&self) -> Option<usize> {
        let quote = match (self.text[self.cursor], self.text.get(self.cursor + 1)) {
            ('"', _) => self.cursor,
            ('b' | 'c', Some('"')) => self.cursor + 1,
            _ => return None,
        };
        let mut index = quote + 1;
        while let Some(&character) = self.text.get(index) {
            match character {
                '"' => return Some(index + 1),
                '\\' if index + 1 < self.text.len() => index += 2,
                '\\' => return None,
                _ => index += 1,
            }
        }
        None
    }

    /// `b?'(?:\\u\{[0-9A-Fa-f_]+\}|\\x[0-9A-Fa-f]{2}|\\.|[^'\\\n])'`: each body in pattern order,
    /// then the closing quote.
    fn character_end(&self) -> Option<usize> {
        let quote = match (self.text[self.cursor], self.text.get(self.cursor + 1)) {
            ('\'', _) => self.cursor,
            ('b', Some('\'')) => self.cursor + 1,
            _ => return None,
        };
        let body = quote + 1;
        [
            self.unicode_escape_end(body),
            self.hex_escape_end(body),
            self.escape_end(body),
            self.plain_character_end(body),
        ]
        .into_iter()
        .flatten()
        .find(|&end| self.text.get(end) == Some(&'\''))
        .map(|end| end + 1)
    }

    /// `\\u\{[0-9A-Fa-f_]+\}`
    fn unicode_escape_end(&self, start: usize) -> Option<usize> {
        if !self.starts_with(start, &['\\', 'u', '{']) {
            return None;
        }
        let digits_end = self.run_end(start + 3, |character| {
            character.is_ascii_hexdigit() || character == '_'
        });
        (digits_end > start + 3 && self.text.get(digits_end) == Some(&'}'))
            .then_some(digits_end + 1)
    }

    /// `\\x[0-9A-Fa-f]{2}`
    fn hex_escape_end(&self, start: usize) -> Option<usize> {
        match self.text.get(start..start + 4) {
            Some(['\\', 'x', high, low]) if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() => {
                Some(start + 4)
            },
            _ => None,
        }
    }

    /// `\\.`, with `.` matching a newline.
    fn escape_end(&self, start: usize) -> Option<usize> {
        (self.text.get(start) == Some(&'\\') && start + 1 < self.text.len()).then_some(start + 2)
    }

    /// `[^'\\\n]`
    fn plain_character_end(&self, start: usize) -> Option<usize> {
        self.text
            .get(start)
            .filter(|character| !matches!(character, '\'' | '\\' | '\n'))
            .map(|_| start + 1)
    }

    /// Python `NUMBER`: `\d[\w.]*(?:[+-]\d+)?`.
    fn number(&self) -> Option<Step> {
        if !in_table(DECIMAL, self.text[self.cursor]) {
            return None;
        }
        let body_end = self.run_end(self.cursor + 1, |character| {
            character == '.' || in_table(WORD, character)
        });
        let exponent = matches!(self.text.get(body_end), Some('+' | '-'))
            && self
                .text
                .get(body_end + 1)
                .is_some_and(|&character| in_table(DECIMAL, character));
        let end = if exponent {
            self.run_end(body_end + 1, |character| in_table(DECIMAL, character))
        } else {
            body_end
        };
        Some(Step::Token {
            end,
            kind: TokenKind::Literal,
        })
    }

    /// Python `IDENTIFIER`: `(?:r#)?[A-Za-z_][A-Za-z_0-9]*`. The words `true` and `false` are
    /// literals.
    fn identifier(&self) -> Option<Step> {
        let starts_word = |index: usize| {
            self.text
                .get(index)
                .is_some_and(|&character| character.is_ascii_alphabetic() || character == '_')
        };
        let word_start = Some(self.cursor + 2)
            .filter(|&start| self.starts_with(self.cursor, &['r', '#']) && starts_word(start))
            .or_else(|| Some(self.cursor).filter(|&start| starts_word(start)))?;
        let end = self.run_end(word_start + 1, |character| {
            character.is_ascii_alphanumeric() || character == '_'
        });
        let kind = match &self.text[self.cursor..end] {
            ['t', 'r', 'u', 'e'] | ['f', 'a', 'l', 's', 'e'] => TokenKind::Literal,
            _ => TokenKind::Identifier,
        };
        Some(Step::Token { end, kind })
    }

    /// Python `PUNCTUATION`: the pairs in pattern order, then one character of the class.
    fn punctuation(&self) -> Option<Step> {
        let length = PUNCTUATION_PAIRS
            .iter()
            .find(|pair| self.starts_with(self.cursor, *pair))
            .map(|pair| pair.len())
            .or_else(|| {
                PUNCTUATION_SINGLES
                    .contains(self.text[self.cursor])
                    .then_some(1)
            })?;
        Some(Step::Token {
            end:  self.cursor + length,
            kind: TokenKind::Punctuation,
        })
    }

    fn starts_with(&self, start: usize, pattern: &[char]) -> bool {
        self.text.get(start..start + pattern.len()) == Some(pattern)
    }

    /// Python `str.find`: the first offset at or after start where pattern starts.
    fn find(&self, start: usize, pattern: &[char]) -> Option<usize> {
        self.text
            .get(start..)?
            .windows(pattern.len())
            .position(|window| window == pattern)
            .map(|offset| start + offset)
    }

    /// The offset after the run of characters from start for which test holds.
    fn run_end(&self, start: usize, test: impl Fn(char) -> bool) -> usize {
        self.text.get(start..).map_or(start, |rest| {
            start
                + rest
                    .iter()
                    .take_while(|&&character| test(character))
                    .count()
        })
    }
}

/// Read tokens and comments without a second scan of the comment bodies.
pub(crate) fn read_with_comments(text: &[char]) -> TokenReading { Tokenizer::new(text).read() }

/// Python `tokenized_text`: the code tokens of text.
#[cfg(test)]
pub(crate) fn tokenize(text: &[char]) -> Vec<Token> { Tokenizer::new(text).run() }

/// Reuse unchanged tokens and gaps; read changed regions until their token boundaries align.
pub(crate) fn normalized_tokens(
    text: &[char],
    normalized: &[char],
    tokens: &[Token],
) -> Vec<Token> {
    let mut position = 0;
    let mut changed = Vec::new();
    let offsets: Vec<_> = text
        .iter()
        .enumerate()
        .map(|(index, &character)| {
            let offset = position.min(normalized.len());
            if normalized.get(offset) != Some(&character) {
                changed.push(index);
            }
            if character != '\r' || text.get(index + 1) != Some(&'\n') {
                position += 1;
            }
            offset
        })
        .chain(std::iter::once(normalized.len()))
        .collect();
    let unchanged = |start: usize, end: usize| {
        changed
            .get(changed.partition_point(|&index| index < start))
            .is_none_or(|&index| index >= end)
    };
    let mut reused = Vec::new();
    let mut previous_end = 0;
    for token in tokens {
        if token.kind == TokenKind::Unknown && token.value == UNTERMINATED_COMMENT {
            if unchanged(previous_end, text.len()) {
                reused.push(UnchangedStep {
                    start: offsets[previous_end],
                    step:  Step::UnterminatedComment,
                });
            }
            previous_end = text.len();
            break;
        }
        if offsets[previous_end] < offsets[token.start] && unchanged(previous_end, token.start) {
            reused.push(UnchangedStep {
                start: offsets[previous_end],
                step:  Step::Skip {
                    end: offsets[token.start],
                },
            });
        }
        let end = (token.start + token.value.chars().count()).min(text.len());
        if unchanged(token.start, end) {
            reused.push(UnchangedStep {
                start: offsets[token.start],
                step:  Step::Token {
                    end:  offsets[end],
                    kind: token.kind,
                },
            });
        }
        previous_end = end;
    }
    if offsets[previous_end] < normalized.len() && unchanged(previous_end, text.len()) {
        reused.push(UnchangedStep {
            start: offsets[previous_end],
            step:  Step::Skip {
                end: normalized.len(),
            },
        });
    }
    let mut reader = Tokenizer::new(normalized);
    reader.reused = &reused;
    reader.run()
}

/// Whether character falls in one of the sorted inclusive ranges of table.
pub(crate) fn in_table(table: &[(u32, u32)], character: char) -> bool {
    let code_point = u32::from(character);
    let index = table.partition_point(|&(_, end)| end < code_point);
    table
        .get(index)
        .is_some_and(|&(start, _)| start <= code_point)
}

#[cfg(test)]
mod tests {
    use super::Token;
    use super::TokenKind;
    use super::read_with_comments;
    use super::tokenize;
    use crate::constants::UNTERMINATED_COMMENT;

    fn read(text: &str) -> Vec<(String, TokenKind, usize, usize)> {
        let characters: Vec<char> = text.chars().collect();
        tokenize(&characters)
            .into_iter()
            .map(
                |Token {
                     value,
                     kind,
                     line,
                     start,
                 }| (value, kind, line, start),
            )
            .collect()
    }

    fn token(
        value: &str,
        kind: TokenKind,
        line: usize,
        start: usize,
    ) -> (String, TokenKind, usize, usize) {
        (value.to_owned(), kind, line, start)
    }

    #[test]
    fn comments_drop_out_and_lines_count_newlines() {
        assert_eq!(
            read("a // b\n/* c /* d */ */ e"),
            vec![
                token("a", TokenKind::Identifier, 1, 0),
                token("e", TokenKind::Identifier, 2, 23),
            ]
        );
        let text: Vec<_> = "a // b\n/* c /* d */ */ e".chars().collect();
        let reading = read_with_comments(&text);
        assert_eq!(
            reading
                .comments
                .iter()
                .map(|comment| (
                    comment.token_position,
                    text[comment.start..comment.end].iter().collect::<String>()
                ))
                .collect::<Vec<_>>(),
            vec![(1, "// b".to_owned()), (1, "/* c /* d */ */".to_owned())]
        );
    }

    #[test]
    fn an_unterminated_comment_is_one_unknown_token_at_the_end() {
        assert_eq!(
            read("a\n/* b"),
            vec![
                token("a", TokenKind::Identifier, 1, 0),
                token(UNTERMINATED_COMMENT, TokenKind::Unknown, 2, 6),
            ]
        );
    }

    #[test]
    fn python_space_includes_the_separator_controls() {
        assert_eq!(
            read("a\u{1c}b\u{85}c"),
            vec![
                token("a", TokenKind::Identifier, 1, 0),
                token("b", TokenKind::Identifier, 1, 2),
                token("c", TokenKind::Identifier, 1, 4),
            ]
        );
    }

    #[test]
    fn offsets_count_code_points() {
        assert_eq!(
            read("\"é\" x"),
            vec![
                token("\"é\"", TokenKind::Literal, 1, 0),
                token("x", TokenKind::Identifier, 1, 4),
            ]
        );
    }

    #[test]
    fn raw_strings_end_at_their_hashes() {
        assert_eq!(
            read("br##\"a\"#\"## r\"x\" r#y"),
            vec![
                token("br##\"a\"#\"##", TokenKind::Literal, 1, 0),
                token("r\"x\"", TokenKind::Literal, 1, 12),
                token("r#y", TokenKind::Identifier, 1, 17),
            ]
        );
    }

    #[test]
    fn an_unterminated_raw_string_runs_to_the_end() {
        assert_eq!(
            read("x r#\"a\"\n"),
            vec![
                token("x", TokenKind::Identifier, 1, 0),
                token("r#\"a\"\n", TokenKind::Unknown, 1, 2),
            ]
        );
    }

    #[test]
    fn character_literals_follow_the_pattern_order() {
        assert_eq!(
            read(r"'\u{41}' '\x41' '\n' 'a' b'z'"),
            vec![
                token(r"'\u{41}'", TokenKind::Literal, 1, 0),
                token(r"'\x41'", TokenKind::Literal, 1, 9),
                token(r"'\n'", TokenKind::Literal, 1, 16),
                token("'a'", TokenKind::Literal, 1, 21),
                token("b'z'", TokenKind::Literal, 1, 25),
            ]
        );
    }

    #[test]
    fn a_lifetime_is_a_quote_and_an_identifier() {
        assert_eq!(
            read("&'a"),
            vec![
                token("&", TokenKind::Punctuation, 1, 0),
                token("'", TokenKind::Punctuation, 1, 1),
                token("a", TokenKind::Identifier, 1, 2),
            ]
        );
    }

    #[test]
    fn an_unterminated_string_is_an_unknown_quote() {
        assert_eq!(
            read("\"a"),
            vec![
                token("\"", TokenKind::Unknown, 1, 0),
                token("a", TokenKind::Identifier, 1, 1),
            ]
        );
    }

    #[test]
    fn numbers_take_word_characters_and_one_exponent() {
        assert_eq!(
            read("1.5e-3x 0x_ff ٣٤"),
            vec![
                token("1.5e-3", TokenKind::Literal, 1, 0),
                token("x", TokenKind::Identifier, 1, 6),
                token("0x_ff", TokenKind::Literal, 1, 8),
                token("٣٤", TokenKind::Literal, 1, 14),
            ]
        );
    }

    #[test]
    fn booleans_are_literals_and_pairs_come_first() {
        assert_eq!(
            read("true::r#false=>`"),
            vec![
                token("true", TokenKind::Literal, 1, 0),
                token("::", TokenKind::Punctuation, 1, 4),
                token("r#false", TokenKind::Identifier, 1, 6),
                token("=>", TokenKind::Punctuation, 1, 13),
                token("`", TokenKind::Unknown, 1, 15),
            ]
        );
    }
}

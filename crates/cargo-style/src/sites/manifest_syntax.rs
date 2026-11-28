//! Accept the manifest syntax of the pinned Python TOML reader.

use toml::Table;
use toml::Value;
use toml::de::Error;
use toml::value::Offset;
use toml_parser::Source;
use toml_parser::lexer::TokenKind;

enum Container {
    Array,
    InlineTable,
}

pub(crate) fn read_toml_1_0(text: &str) -> Result<Table, Error> {
    if supports_tokens(text) {
        let table = text.parse::<Table>()?;
        if table.values().all(supports_value) {
            return Ok(table);
        }
    }
    Err(serde::de::Error::custom("expected TOML 1.0 syntax"))
}

pub(super) fn manifest_table(text: &str) -> Option<Table> { read_toml_1_0(text).ok() }

fn supports_tokens(text: &str) -> bool {
    if text.starts_with('\u{feff}')
        || text
            .match_indices('\r')
            .any(|(offset, _)| !text[offset + 1..].starts_with('\n'))
    {
        return false;
    }
    let mut containers = Vec::new();
    let mut previous = TokenKind::Eof;
    for token in Source::new(text).lex() {
        let kind = token.kind();
        match kind {
            TokenKind::Whitespace => continue,
            TokenKind::LeftSquareBracket => containers.push(Container::Array),
            TokenKind::LeftCurlyBracket => containers.push(Container::InlineTable),
            TokenKind::RightCurlyBracket if previous == TokenKind::Comma => return false,
            TokenKind::RightSquareBracket | TokenKind::RightCurlyBracket => {
                let _ = containers.pop();
            },
            TokenKind::Newline | TokenKind::Comment
                if matches!(containers.last(), Some(Container::InlineTable)) =>
            {
                return false;
            },
            TokenKind::BasicString | TokenKind::MlBasicString => {
                let span = token.span();
                if has_new_escape(&text[span.start()..span.end()]) {
                    return false;
                }
            },
            _ => {},
        }
        previous = kind;
    }
    true
}

fn has_new_escape(text: &str) -> bool {
    let mut bytes = text.bytes();
    while bytes.by_ref().any(|byte| byte == b'\\') {
        if matches!(bytes.next(), Some(b'e' | b'x')) {
            return true;
        }
    }
    false
}

fn supports_value(value: &Value) -> bool {
    match value {
        Value::Datetime(datetime) => {
            datetime.date.is_none_or(|date| date.year > 0)
                && datetime
                    .time
                    .is_none_or(|time| time.second.is_some_and(|seconds| seconds < 60))
                && !matches!(datetime.offset, Some(Offset::Custom { minutes }) if minutes.unsigned_abs() >= 24 * 60)
        },
        Value::Array(values) => values.iter().all(supports_value),
        Value::Table(table) => table.values().all(supports_value),
        _ => true,
    }
}

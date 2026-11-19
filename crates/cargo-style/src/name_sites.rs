//! The code identifiers that name each declared item inside its normalized body.

use std::borrow::Cow;

use serde::Serialize;

use crate::declarations::DeclaredFile;
use crate::declarations::ItemBody;
use crate::declarations::ItemSpan;
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::unicode_tables::LINE_BOUNDARIES;
use crate::unicode_tables::SPACE;

/// The item has no readable body or its source has unknown tokens.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnreadableBody {
    Unreadable,
}

/// A body's name offsets, or the Python unreadable-body result.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum NameSiteReading {
    Unreadable(UnreadableBody),
    Sites { sites: Vec<[usize; 2]> },
}

/// Shared normalized tokens and each declaration's name sites.
pub(crate) struct BodyFacts {
    pub(crate) tokens: BodyTokens,
    pub(crate) sites:  Vec<NameSiteReading>,
}

/// Reuse original offsets, retain changed tokens, or report an unreadable body.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum BodyTokens {
    State(BodyTokenState),
    Tokens(Vec<Token>),
}

/// The normalized body needs no new token list, or cannot supply offsets.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BodyTokenState {
    Original,
    Unreadable,
}

/// Python `str.splitlines`, excluding separators and a final empty line.
fn source_lines(source: &[char]) -> Vec<&[char]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    while cursor < source.len() {
        if tokenizer::in_table(LINE_BOUNDARIES, source[cursor]) {
            lines.push(&source[start..cursor]);
            if source[cursor] == '\r' && source.get(cursor + 1) == Some(&'\n') {
                cursor += 1;
            }
            start = cursor + 1;
        }
        cursor += 1;
    }
    if start < source.len() {
        lines.push(&source[start..]);
    }
    lines
}

/// Join lines with one newline, as the Python body reader does.
fn joined_lines<'a>(lines: impl IntoIterator<Item = &'a [char]>) -> Vec<char> {
    let mut result = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        result.extend_from_slice(line);
    }
    result
}

const fn unreadable() -> NameSiteReading { NameSiteReading::Unreadable(UnreadableBody::Unreadable) }

/// Read one body after the file's line normalization.
fn body_sites(source: &[char], tokens: &[Token], span: &ItemSpan, name: &str) -> NameSiteReading {
    let ItemBody::Braced(body) = &span.body else {
        return unreadable();
    };
    let Some(closing) = tokens.get(body.close) else {
        return unreadable();
    };
    let Some(opening) = tokens.get(body.open) else {
        return unreadable();
    };
    let start = opening.start + opening.value.chars().count();
    let Some(body_source) = source.get(start..closing.start) else {
        return unreadable();
    };
    let mut source_offset = start;
    let mut body_offset = 0;
    let mut sites = Vec::new();
    let mut identifiers = tokens[body.open + 1..body.close]
        .iter()
        .filter(|token| token.kind == TokenKind::Identifier && token.value == name)
        .peekable();
    for line in body_source.split(|&character| character == '\n') {
        let end = source_offset + line.len();
        if line
            .iter()
            .any(|&character| !tokenizer::in_table(SPACE, character))
        {
            while let Some(token) = identifiers.next_if(|token| token.start < end) {
                let offset = body_offset + token.start - source_offset;
                sites.push([offset, offset + token.value.chars().count()]);
            }
            body_offset += line.len() + 1;
        }
        source_offset = end + 1;
    }
    NameSiteReading::Sites { sites }
}

/// Reuse blob tokens when normalization changes offsets but keeps lexical boundaries.
fn normalized_tokens<'a>(
    text: &[char],
    normalized: &[char],
    tokens: &'a [Token],
) -> Cow<'a, [Token]> {
    if text.starts_with(normalized)
        && text[normalized.len()..]
            .iter()
            .all(|&character| character == '\n')
    {
        return Cow::Borrowed(tokens);
    }
    Cow::Owned(tokenizer::normalized_tokens(text, normalized, tokens))
}

/// Read every declared item's body, in declaration order.
pub(crate) fn read_facts(text: &[char], file: &DeclaredFile) -> BodyFacts {
    let normalized = joined_lines(source_lines(text));
    let tokens = normalized_tokens(text, &normalized, &file.tokens);
    if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
        return BodyFacts {
            tokens: BodyTokens::State(BodyTokenState::Unreadable),
            sites:  file.items.iter().map(|_| unreadable()).collect(),
        };
    }
    let sites = file
        .items
        .iter()
        .map(|item| {
            let first = file.items.iter().position(|candidate| candidate == item);
            first
                .and_then(|index| file.spans.get(index))
                .map_or_else(unreadable, |span| {
                    body_sites(&normalized, &tokens, span, &item.name)
                })
        })
        .collect();
    BodyFacts {
        sites,
        tokens: match tokens {
            Cow::Borrowed(_) => BodyTokens::State(BodyTokenState::Original),
            Cow::Owned(tokens) => BodyTokens::Tokens(tokens),
        },
    }
}

/// Read only name sites for the existing unit tests.
#[cfg(test)]
fn read(text: &[char], file: &DeclaredFile) -> Vec<NameSiteReading> { read_facts(text, file).sites }

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::NameSiteReading;
    use super::read;
    use super::unreadable;
    use crate::declarations;

    #[test]
    fn names_count_code_points_and_exclude_comments_and_literals() -> Result<(), Box<dyn Error>> {
        let text: Vec<char> = "fn work() { let text = \"é\"; work(); /* work */ \"work\"; }"
            .chars()
            .collect();
        let file = declarations::read(&text);
        let body = " let text = \"é\"; work(); /* work */ \"work\"; ";
        let start = body.find("work()").ok_or("the code name is missing")?;
        let point = body[..start].chars().count();
        assert_eq!(
            read(&text, &file),
            [NameSiteReading::Sites {
                sites: vec![[point, point + "work".len()]],
            }]
        );
        Ok(())
    }

    #[test]
    fn normalization_keeps_comment_bodies_and_rejects_unreadable_bodies() {
        for (source, expected) in [
            (
                "fn work() {\r\n\r\nwork();\r\n}\r\n",
                NameSiteReading::Sites {
                    sites: vec![[0, 4]],
                },
            ),
            (
                "fn work() { // work\n /* work */\n}",
                NameSiteReading::Sites { sites: Vec::new() },
            ),
            ("fn work();", unreadable()),
            ("fn work() { work();", unreadable()),
            ("fn work() { ` }", unreadable()),
        ] {
            let text: Vec<char> = source.chars().collect();
            assert_eq!(
                read(&text, &declarations::read(&text)),
                [expected],
                "{source:?}"
            );
        }
    }
}

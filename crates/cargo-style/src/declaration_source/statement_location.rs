//! Find the line that carries a statement in a moved function.

use std::ops::Range;

use serde::Serialize;

use super::DeclarationFile;
use crate::declarations::DeclarationAt;
use crate::declarations::DeclaredItem;
use crate::declarations::ItemBody;
use crate::source_lines::SiteLine;
use crate::source_sites;
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;

#[derive(Serialize)]
pub(crate) enum StatementRefusal {
    #[serde(rename = "text not on a statement line of the moved function")]
    TextNotFound,
    #[serde(rename = "text on more than one line of the moved function's body")]
    TextRepeated,
}

pub(crate) enum StatementLocation {
    Matched(usize),
    Refused(StatementRefusal),
}

pub(super) enum StatementTokens {
    Original,
    Normalized(Vec<Token>),
}

enum StatementLines {
    Proved(Range<usize>),
    NoProvedBody,
}

impl DeclarationFile {
    pub(crate) fn owns_statement(&self, item: &DeclaredItem, line: usize) -> bool {
        matches!(self.statement_lines(item), StatementLines::Proved(lines) if lines.contains(&line))
    }

    fn statement_lines(&self, item: &DeclaredItem) -> StatementLines {
        if item.keyword() != "fn" {
            return StatementLines::NoProvedBody;
        }
        let ItemBody::Braced(body) = self.body(item) else {
            return StatementLines::NoProvedBody;
        };
        let start = self.word_lines[body.open]
            .max(*item.lines().start())
            .max(item.keyword_line());
        let Some(&close) = self.word_lines.get(body.close) else {
            return StatementLines::NoProvedBody;
        };
        let end = close.min(item.lines().end().saturating_add(1));
        StatementLines::Proved(start.saturating_add(1)..end)
    }

    pub(crate) fn moved_line(
        &self,
        item: &DeclaredItem,
        original: &Self,
        old: &DeclaredItem,
        line: usize,
    ) -> usize {
        let SiteLine::Text(text) = original.source.lines().line(line) else {
            return item.keyword_line();
        };
        let text = source_sites::trim(text);
        if text.is_empty() {
            return item.keyword_line();
        }
        let nested: Vec<_> = self
            .items()
            .iter()
            .filter(|candidate| {
                *candidate != item
                    && item.lines().contains(candidate.lines().start())
                    && item.lines().contains(candidate.lines().end())
            })
            .map(DeclaredItem::lines)
            .collect();
        let offset = line as i128 - old.keyword_line() as i128;
        let lines = self.source.lines();
        (*item.lines().start()..=(*item.lines().end()).min(lines.count()))
            .filter(|&number| matches!(lines.line(number), SiteLine::Text(text_at_line) if source_sites::trim(text_at_line) == text)
                && !nested.iter().any(|lines| lines.contains(&number)))
            .min_by_key(|&number| (number as i128 - item.keyword_line() as i128 - offset).abs())
            .unwrap_or_else(|| item.keyword_line())
    }

    pub(crate) fn statement_location(&self, item: &DeclaredItem, text: &str) -> StatementLocation {
        let text = source_sites::trim(text);
        let reject = StatementLocation::Refused(StatementRefusal::TextNotFound);
        if text.contains('\n') {
            return reject;
        }
        let StatementLines::Proved(body) = self.statement_lines(item) else {
            return reject;
        };
        let wanted = Self::read_tokens(text);
        let tokens = self.statement_tokens();
        if wanted.is_empty()
            || wanted
                .iter()
                .chain(tokens)
                .any(|token| token.kind == TokenKind::Unknown)
        {
            return reject;
        }
        let mut matches = tokens
            .chunk_by(|a, b| a.line == b.line)
            .filter_map(|tokens| {
                let number = tokens[0].line;
                (body.contains(&number)
                    && tokens.windows(wanted.len()).any(|run| {
                        run.iter()
                            .zip(&wanted)
                            .all(|(a, b)| a.kind == b.kind && a.value == b.value)
                    }))
                .then_some(number)
            });
        let Some(number) = matches.next() else {
            return reject;
        };
        if matches.next().is_some() {
            return StatementLocation::Refused(StatementRefusal::TextRepeated);
        }
        if matches!(self.at(number), DeclarationAt::Named(owner) if owner == item) {
            StatementLocation::Matched(number)
        } else {
            reject
        }
    }

    fn statement_tokens(&self) -> &[Token] {
        let original = &self.source.syntax().tokens;
        let tokens = self.statements.get_or_init(|| {
            let normalized = self.normalized_text();
            let text = self.lines().text();
            if text == normalized || text.strip_suffix('\n') == Some(normalized) {
                return StatementTokens::Original;
            }
            let crlf = text.replace("\r\n", "\n");
            let tokens = if crlf == normalized || crlf.strip_suffix('\n') == Some(normalized) {
                tokenizer::normalized_tokens(
                    &text.chars().collect::<Vec<_>>(),
                    &normalized.chars().collect::<Vec<_>>(),
                    original,
                )
            } else {
                Self::read_tokens(normalized)
            };
            StatementTokens::Normalized(tokens)
        });
        match tokens {
            StatementTokens::Original => original,
            StatementTokens::Normalized(tokens) => tokens,
        }
    }
}

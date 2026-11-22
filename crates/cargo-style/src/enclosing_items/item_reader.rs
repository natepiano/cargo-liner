//! Read complete item identities and the lines whose owners are proved.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use super::nested_syntax::NestedSyntax;
use crate::source_lines::SourceLines;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::word_syntax;

pub(crate) struct EnclosingItem {
    pub(crate) identity: Vec<String>,
    pub(crate) kind:     String,
    pub(crate) lines:    RangeInclusive<usize>,
    pub(crate) header:   Vec<String>,
    pub(crate) body:     OwnerBody,
}

pub(crate) enum OwnerBody {
    Proved,
    Unproved(RangeInclusive<usize>),
}

pub(crate) struct UnreadableItems;

pub(crate) fn read(
    source: &SourceLines,
    tokens: &[Token],
) -> Result<Vec<EnclosingItem>, UnreadableItems> {
    if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
        return Err(UnreadableItems);
    }
    let words: Vec<_> = tokens.iter().map(|token| token.value.clone()).collect();
    let ends = word_syntax::delimiter_ends(&words);
    let openers = ends.iter().map(|(&open, &close)| (close, open)).collect();
    let attributes = word_syntax::attribute_spans(&words).into_iter().collect();
    let mut count = 0;
    let offsets = std::iter::once(0)
        .chain(source.lines().map(|line| {
            count += line.chars().count();
            count
        }))
        .collect();
    let mut reader = ItemReader {
        tokens,
        words,
        ends,
        openers,
        attributes,
        offsets,
        items: Vec::new(),
    };
    if !reader.visit(0, tokens.len(), &[]) {
        return Err(UnreadableItems);
    }
    Ok(reader.items)
}

pub(super) struct ItemReader<'a> {
    pub(super) tokens:  &'a [Token],
    words:              Vec<String>,
    ends:               BTreeMap<usize, usize>,
    pub(super) openers: BTreeMap<usize, usize>,
    attributes:         BTreeMap<usize, usize>,
    offsets:            Vec<usize>,
    items:              Vec<EnclosingItem>,
}

struct ItemSyntax {
    first:   usize,
    keyword: usize,
    last:    usize,
    kind:    String,
    title:   String,
}

impl ItemReader<'_> {
    pub(super) fn record_nested_item(
        &mut self,
        first: usize,
        last: usize,
        owner: &[String],
    ) -> bool {
        let Some(item) = self.item(first, last).filter(|item| item.last == last) else {
            return false;
        };
        self.record(&item, owner);
        true
    }

    pub(super) fn word(&self, index: usize) -> &str { word_syntax::word(&self.words, index) }

    pub(super) fn is_identifier(&self, index: usize) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Identifier)
    }

    pub(super) fn close(&self, opening: usize, end: usize) -> Option<usize> {
        self.ends
            .get(&opening)
            .copied()
            .filter(|&close| close < end)
    }

    pub(super) fn skip_attributes(&self, mut cursor: usize, end: usize) -> usize {
        while cursor < end {
            let Some(&next) = self.attributes.get(&cursor) else {
                break;
            };
            cursor = next.min(end);
        }
        cursor
    }

    pub(super) fn item_prefix(&self, mut cursor: usize, end: usize) -> usize {
        while cursor < end {
            let following = self.word(cursor + 1);
            match self.word(cursor) {
                "pub" if following == "(" => {
                    cursor = self.close(cursor + 1, end).map_or(end, |close| close + 1);
                },
                "pub" | "default" | "async" | "unsafe" | "safe" | "auto" => cursor += 1,
                "const"
                    if matches!(
                        following,
                        "pub"
                            | "default"
                            | "const"
                            | "async"
                            | "unsafe"
                            | "safe"
                            | "extern"
                            | "auto"
                            | "fn"
                    ) =>
                {
                    cursor += 1;
                },
                "extern" => {
                    let next = cursor + 1 + usize::from(following.starts_with('"'));
                    if next < end && matches!(self.word(next), "{" | "crate") {
                        break;
                    }
                    cursor = next;
                },
                _ => break,
            }
        }
        cursor
    }

    pub(super) fn closed_prefix(&self, start: usize, keyword: usize) -> bool {
        let mut cursor = self.skip_attributes(start, keyword);
        if cursor < keyword && self.word(cursor) == "pub" {
            cursor += 1;
            if self.word(cursor) == "(" {
                let Some(close) = self.close(cursor, keyword) else {
                    return false;
                };
                if !self.closed_visibility(cursor + 1, close) {
                    return false;
                }
                cursor = close + 1;
            }
        }
        for word in ["default", "const", "async", "unsafe", "extern"] {
            if cursor < keyword && self.word(cursor) == word {
                cursor += 1;
                if word == "extern"
                    && cursor < keyword
                    && self.tokens[cursor].kind == TokenKind::Literal
                {
                    cursor += 1;
                }
            }
        }
        cursor == keyword
    }

    fn closed_visibility(&self, start: usize, end: usize) -> bool {
        if start + 1 == end && matches!(self.word(start), "crate" | "self" | "super") {
            return true;
        }
        self.word(start) == "in"
            && (end - start).is_multiple_of(2)
            && end > start + 1
            && (start + 1..end).enumerate().all(|(index, token)| {
                if index % 2 == 0 {
                    word_syntax::identifier(self.word(token))
                } else {
                    self.word(token) == "::"
                }
            })
    }

    pub(super) fn item_end(&self, mut cursor: usize, end: usize, body: bool) -> Option<usize> {
        while cursor < end {
            match self.word(cursor) {
                ";" => return Some(cursor + 1),
                "{" if body => return self.close(cursor, end).map(|close| close + 1),
                "{" | "(" | "[" => cursor = self.close(cursor, end)? + 1,
                _ => cursor += 1,
            }
        }
        None
    }

    fn item(&self, start: usize, end: usize) -> Option<ItemSyntax> {
        let keyword = self.item_prefix(self.skip_attributes(start, end), end);
        if keyword >= end {
            return None;
        }
        let word_at = |index| if index < end { self.word(index) } else { "" };
        let word = word_at(keyword);
        let following = word_at(keyword + 1);
        let (kind, name, last) = match word {
            "use" => ("use", "", self.item_end(keyword, end, false)?),
            "const" | "type" | "static" => {
                let name = if word == "static" && following == "mut" {
                    word_at(keyword + 2)
                } else {
                    following
                };
                (word, name, self.item_end(keyword, end, false)?)
            },
            "extern" if following == "crate" => (
                "extern crate",
                word_at(keyword + 2),
                self.item_end(keyword, end, false)?,
            ),
            "impl" | "extern" => (word, "", self.item_end(keyword + 1, end, true)?),
            "fn" | "struct" | "enum" | "union" | "trait" => {
                (word, following, self.item_end(keyword + 1, end, true)?)
            },
            "mod" => (word, following, self.item_end(keyword, end, true)?),
            "macro_rules" if following == "!" => (
                word,
                word_at(keyword + 2),
                self.item_end(keyword + 3, end, true)?,
            ),
            _ if word_syntax::identifier(word) => ("macro", word, self.macro_end(keyword, end)?),
            _ => return None,
        };
        if !name.is_empty() && !word_syntax::identifier(name) {
            return None;
        }
        let title = if kind == "impl" {
            self.words[keyword..self.openers.get(&(last - 1)).copied().unwrap_or(last)].join(" ")
        } else {
            format!("{kind} {name}")
        };
        Some(ItemSyntax {
            first: start,
            keyword,
            last,
            kind: kind.to_owned(),
            title,
        })
    }

    fn macro_end(&self, mut name: usize, end: usize) -> Option<usize> {
        while name + 2 < end
            && self.word(name + 1) == "::"
            && word_syntax::identifier(self.word(name + 2))
        {
            name += 2;
        }
        if self.word(name + 1) != "!" {
            return None;
        }
        let opening = name + 2;
        let last = self.close(opening, end)? + 1;
        if self.word(opening) == "{" {
            Some(last)
        } else {
            (last < end && self.word(last) == ";").then_some(last + 1)
        }
    }

    fn visit(&mut self, start: usize, end: usize, owner: &[String]) -> bool {
        let mut cursor = start;
        while cursor < end {
            if let Some(&next) = self.attributes.get(&cursor)
                && self.word(cursor + 1) == "!"
            {
                cursor = next;
                continue;
            }
            let Some(item) = self.item(cursor, end) else {
                return false;
            };
            cursor = item.last;
            self.record(&item, owner);
        }
        true
    }

    fn first_line(&self, index: usize) -> usize {
        self.offsets
            .partition_point(|&offset| offset <= self.tokens[index].start)
    }

    fn last_line(&self, index: usize) -> usize {
        let token = &self.tokens[index];
        let end = token.start + token.value.chars().count().saturating_sub(1);
        self.offsets.partition_point(|&offset| offset <= end)
    }

    fn record(&mut self, item: &ItemSyntax, owner: &[String]) {
        let mut identity = owner.to_vec();
        identity.push(item.title.clone());
        let lines = self.first_line(item.first)..=self.last_line(item.last - 1);
        let position = self.items.len();
        self.items.push(EnclosingItem {
            identity: identity.clone(),
            kind:     item.kind.clone(),
            lines:    lines.clone(),
            header:   Vec::new(),
            body:     OwnerBody::Unproved(lines),
        });
        if self.closed_prefix(item.first, item.keyword) {
            self.read_body(item, &identity, position);
        }
        if matches!(self.items[position].body, OwnerBody::Unproved(_)) {
            self.items.truncate(position + 1);
        }
    }

    fn read_body(&mut self, item: &ItemSyntax, owner: &[String], position: usize) {
        if matches!(item.kind.as_str(), "macro_rules" | "macro") {
            let opening = item.keyword + if item.kind == "macro_rules" { 3 } else { 2 };
            if let Some(close) = self.close(opening, item.last) {
                self.items[position].header = self.words[item.first..opening].to_vec();
                self.items[position].body =
                    OwnerBody::Unproved(self.first_line(opening)..=self.last_line(close));
            }
            return;
        }
        let mut cursor = item.keyword + 1;
        while cursor < item.last {
            let opening = self.word(cursor);
            if !matches!(opening, "{" | "(" | "[") {
                cursor += 1;
                continue;
            }
            let Some(close) = self.close(cursor, item.last) else {
                return;
            };
            let syntax = match (opening, item.kind.as_str()) {
                ("(", "fn") => NestedSyntax::Parameters,
                ("{", "struct" | "union") => NestedSyntax::Fields,
                ("{", "enum") => NestedSyntax::Variants,
                ("{", _) => NestedSyntax::Statements,
                _ => NestedSyntax::Continuation,
            };
            let body = opening == "{" && close == item.last - 1;
            if body {
                self.items[position].header = self.words[item.first..cursor].to_vec();
            }
            if body && matches!(item.kind.as_str(), "mod" | "impl" | "trait" | "extern") {
                if self.visit(cursor + 1, close, owner) {
                    self.items[position].body = OwnerBody::Proved;
                }
                return;
            }
            if !self.nested(cursor + 1, close, owner, syntax) {
                return;
            }
            cursor = close + 1;
        }
        if self.items[position].header.is_empty() {
            self.items[position].header = self.words[item.first..item.last].to_vec();
        }
        self.items[position].body = OwnerBody::Proved;
    }
}

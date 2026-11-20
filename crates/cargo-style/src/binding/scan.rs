//! Read the token positions that the binding proof shares.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::Range;

use super::constants::KEYWORDS;
use super::constants::NAMED_ITEMS;
use super::patterns::PatternSite;
use crate::enclosing_items;
use crate::enclosing_items::EnclosingItem;
use crate::enclosing_items::UnreadableItems;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;
use crate::source_sites;
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::unicode_tables::LINE_BOUNDARIES;
use crate::word_syntax;

pub(crate) struct BindingScan {
    pub(super) source:      SourceLines,
    pub(super) chars:       Vec<char>,
    pub(super) tokens:      Vec<Token>,
    pub(super) words:       Vec<String>,
    pub(super) ends:        BTreeMap<usize, usize>,
    pub(super) parents:     BTreeMap<usize, usize>,
    pub(super) attributes:  BTreeMap<usize, usize>,
    pub(super) macros:      Vec<(usize, usize)>,
    pub(super) named_items: Vec<Range<usize>>,
    pub(super) sites:       Vec<PatternSite>,
    pub(super) formats:     BTreeSet<usize>,
    items:                  OnceCell<Result<Vec<EnclosingItem>, UnreadableItems>>,
}

impl BindingScan {
    pub(crate) fn read(source: String) -> Result<Self, UnreadableItems> {
        let chars: Vec<_> = source.chars().collect();
        let tokens = tokenizer::read_with_comments(&chars).tokens;
        if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
            return Err(UnreadableItems::Tokens);
        }
        let words: Vec<_> = tokens.iter().map(|token| token.value.clone()).collect();
        let ends = word_syntax::delimiter_ends(&words);
        let macros = word_syntax::macro_spans(&words, &ends);
        let attributes = word_syntax::attribute_spans(&words).into_iter().collect();
        let parents = parents(words.len(), &ends);
        let mut scan = Self {
            source: source.into(),
            chars,
            tokens,
            words,
            ends,
            parents,
            attributes,
            macros,
            named_items: Vec::new(),
            sites: Vec::new(),
            formats: BTreeSet::new(),
            items: OnceCell::new(),
        };
        scan.named_items = scan.named_items();
        scan.sites = scan.pattern_sites();
        scan.formats = scan.format_arguments();
        Ok(scan)
    }

    pub(super) fn items(&self) -> Result<&[EnclosingItem], UnreadableItems> {
        self.items
            .get_or_init(|| enclosing_items::read(&self.source, &self.tokens))
            .as_deref()
            .map_err(|&error| error)
    }

    pub(super) fn word(&self, index: usize) -> &str { word_syntax::word(&self.words, index) }

    pub(super) fn lead(&self, index: usize) -> &str {
        index
            .checked_sub(1)
            .map_or("", |previous| self.word(previous))
    }

    pub(super) fn level(&self, start: usize, end: usize, angles: bool) -> Vec<usize> {
        let mut result = Vec::new();
        let mut cursor = start;
        let mut depth = 0_usize;
        while cursor < end {
            if angles {
                depth = match self.word(cursor) {
                    "<" => depth + 1,
                    ">" => depth.saturating_sub(1),
                    ">>" => depth.saturating_sub(2),
                    _ => depth,
                };
            }
            if depth == 0 {
                result.push(cursor);
            }
            cursor = self.ends.get(&cursor).copied().unwrap_or(cursor) + 1;
        }
        result
    }

    pub(super) fn stop(&self, start: usize, end: usize, marks: &[&str]) -> usize {
        self.level(start, end, false)
            .into_iter()
            .find(|&index| marks.contains(&self.word(index)))
            .unwrap_or(end)
    }

    pub(super) fn block_end(&self, index: usize) -> usize {
        let mut cursor = index;
        while let Some(&parent) = self.parents.get(&cursor) {
            if self.word(parent) == "{" {
                return self.ends[&parent];
            }
            cursor = parent;
        }
        self.words.len()
    }

    pub(super) fn line_text(&self, line: usize) -> &str {
        if let SiteLine::Text(text) = self.source.line(line) {
            text.trim_end_matches(|character| tokenizer::in_table(LINE_BOUNDARIES, character))
        } else {
            ""
        }
    }

    pub(super) fn has_text(&self, line: usize, expected: &str) -> bool {
        matches!(self.source.line(line), SiteLine::Text(text) if source_sites::trim(text) == source_sites::trim(expected))
    }

    pub(super) fn has_glob(&self) -> bool {
        let mut in_use = false;
        for word in &self.words {
            match word.as_str() {
                "use" => in_use = true,
                ";" => in_use = false,
                "*" if in_use => return true,
                _ => {},
            }
        }
        false
    }

    pub(super) fn token_end(&self, index: usize) -> usize {
        self.tokens[index].start + self.words[index].chars().count()
    }

    pub(super) fn region(&self, tokens: Range<usize>) -> String {
        self.chars[self.tokens[tokens.start].start..self.token_end(tokens.end - 1)]
            .iter()
            .collect()
    }

    pub(super) fn token_offset(&self, index: usize) -> usize { self.tokens[index].start }

    fn named_items(&self) -> Vec<Range<usize>> {
        word_syntax::code_item_starts(&self.words)
            .into_iter()
            .filter(|&index| NAMED_ITEMS.contains(&self.word(index)))
            .map(|index| {
                let end = self.stop(index + 1, self.block_end(index), &["{", ";"]);
                index..self.ends.get(&end).copied().unwrap_or(end) + 1
            })
            .collect()
    }

    fn format_arguments(&self) -> BTreeSet<usize> {
        let mut result = BTreeSet::new();
        for &(bang, finish) in &self.macros {
            let position = match self.lead(bang) {
                "println" | "eprintln" | "print" | "eprint" | "format" | "format_args"
                | "panic" | "unreachable" | "todo" | "unimplemented" => 0,
                "write" | "writeln" | "assert" | "debug_assert" => 1,
                "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => 2,
                _ => continue,
            };
            let mut argument = 0;
            for index in self.level(bang + 2, finish - 1, false) {
                let word = self.word(index);
                if word == "," {
                    argument += 1;
                } else if argument == position
                    && (word.starts_with('"')
                        || word
                            .strip_prefix('r')
                            .is_some_and(|tail| tail.trim_start_matches('#').starts_with('"')))
                    && matches!(self.lead(index), "(" | "[" | "{" | ",")
                {
                    result.insert(index);
                }
            }
        }
        result
    }
}

fn parents(len: usize, ends: &BTreeMap<usize, usize>) -> BTreeMap<usize, usize> {
    let mut stack = Vec::new();
    let mut result = BTreeMap::new();
    for index in 0..len {
        if stack.last().is_some_and(|parent| ends[parent] == index) {
            stack.pop();
        }
        if let Some(&parent) = stack.last() {
            result.insert(index, parent);
        }
        if ends.contains_key(&index) {
            stack.push(index);
        }
    }
    result
}

pub(super) fn plain(word: &str) -> &str { word.strip_prefix("r#").unwrap_or(word) }

pub(crate) fn local_name(word: &str) -> bool {
    word_syntax::identifier(word) && !KEYWORDS.contains(&word)
}

pub(super) fn contains(ranges: &[Range<usize>], index: usize) -> bool {
    ranges.iter().any(|range| range.contains(&index))
}

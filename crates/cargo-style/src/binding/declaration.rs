//! Classify the name at the stored line as a local binding or a nonlocal declaration.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::constants::NAMED_ITEMS;
use super::patterns::BindingScope;
use super::patterns::IdentifierMatch;
use super::patterns::PatternKind;
use super::scan;
use super::scan::BindingScan;
use crate::enclosing_items::EnclosingItem;
use crate::enclosing_items::OwnerBody;
use crate::enclosing_items::OwnerHeader;
use crate::rename_admission::BindingRename;
use crate::rename_admission::RefusalCause;
use crate::tokenizer;
use crate::tokenizer::TokenKind;
use crate::word_syntax;
use crate::word_syntax::MemberKind;

pub(crate) enum Declaration {
    Local(BindingScope),
    NonLocal,
}

enum Candidate {
    Local,
    NonLocal,
}

impl BindingScan {
    pub(crate) fn declare(&self, rename: &BindingRename<'_>) -> Result<Declaration, RefusalCause> {
        let indexes: BTreeSet<_> = self
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(index, token)| {
                (token.line == rename.line && scan::plain(&token.value) == scan::plain(rename.old))
                    .then_some(index)
            })
            .collect();
        let mut candidates = BTreeMap::new();
        for site in &self.sites {
            for &index in &indexes {
                if self.binding_word(site, index, rename.old, IdentifierMatch::WithoutRawPrefix) {
                    candidates.insert(index, Candidate::Local);
                }
            }
        }
        if let Ok(items) = self.items() {
            for item in items {
                self.named_declaration(item, &indexes, &mut candidates);
            }
        }
        if candidates.len() == 1
            && let Some((index, candidate)) = candidates.into_iter().next()
        {
            return match candidate {
                Candidate::Local => self.scope_at(index).map(Declaration::Local),
                Candidate::NonLocal => Ok(Declaration::NonLocal),
            };
        }
        Err(RefusalCause::DeclarationNotUnique(rename.old.to_owned()))
    }

    pub(crate) fn proves_companion(&self, rename: &BindingRename<'_>) -> bool {
        let names = [scan::plain(rename.old), scan::plain(rename.new)];
        !self.words.is_empty()
            && self.macros.is_empty()
            && self.attributes.is_empty()
            && !self.has_glob()
            && !self
                .words
                .iter()
                .any(|word| names.contains(&scan::plain(word)))
            && !tokenizer::read_with_comments(&rename.text.chars().collect::<Vec<_>>())
                .tokens
                .iter()
                .any(|token| {
                    token.kind == TokenKind::Identifier
                        && names.contains(&scan::plain(&token.value))
                })
    }

    fn named_declaration(
        &self,
        item: &EnclosingItem,
        indexes: &BTreeSet<usize>,
        candidates: &mut BTreeMap<usize, Candidate>,
    ) {
        let OwnerHeader::Proved { keyword, .. } = item.header_tokens else {
            return;
        };
        let word = self.word(keyword);
        let name = keyword
            + 1
            + usize::from(
                word == "macro_rules" || word == "static" && self.word(keyword + 1) == "mut",
            );
        if (NAMED_ITEMS.contains(&word) || word == "macro_rules") && indexes.contains(&name) {
            candidates.entry(name).or_insert(Candidate::NonLocal);
        }
        if !matches!(word, "struct" | "enum" | "union") {
            return;
        }
        let OwnerBody::Proved(body) = &item.body else {
            return;
        };
        let start = body.start;
        let Some(&end) = self.ends.get(&start).filter(|_| self.word(start) == "{") else {
            return;
        };
        let positions = if word == "enum" {
            let first_body = (keyword + 2..self.words.len())
                .find(|&index| matches!(self.word(index), "{" | ";"));
            if first_body != Some(start) {
                return;
            }
            self.variant_sites(start, end)
        } else {
            self.field_sites(start, end)
        };
        for index in positions {
            if indexes.contains(&index) {
                candidates.entry(index).or_insert(Candidate::NonLocal);
            }
        }
    }

    fn field_sites(&self, start: usize, end: usize) -> Vec<usize> {
        word_syntax::member_sites(&self.words, start, end, MemberKind::Field)
    }

    fn variant_sites(&self, start: usize, end: usize) -> Vec<usize> {
        word_syntax::member_sites(&self.words, start, end, MemberKind::Variant)
            .into_iter()
            .flat_map(|index| {
                let opening = index + 1;
                let fields = self
                    .ends
                    .get(&opening)
                    .filter(|_| self.word(opening) == "{")
                    .map_or_else(Vec::new, |&close| self.field_sites(opening, close));
                std::iter::once(index).chain(fields)
            })
            .collect()
    }

    pub(super) fn name_word(&self, index: usize) -> bool {
        let word = self.word(index);
        let previous = self.lead(index);
        let following = self.word(index + 1);
        if !word_syntax::identifier(word)
            || matches!(previous, "." | "::" | "'")
            || matches!(following, "::" | "!")
        {
            return false;
        }
        !(following == ":"
            && matches!(previous, "{" | ",")
            && self
                .parents
                .get(&index)
                .is_some_and(|&parent| self.word(parent) == "{")
            && !self.sites.iter().any(|site| {
                matches!(
                    site.kind,
                    PatternKind::Closure | PatternKind::FunctionParameter
                ) && self.binding_word(site, index, word, IdentifierMatch::ExactPlainName)
            }))
    }

    pub(super) fn shorthand(&self, index: usize) -> bool {
        let Some(&parent) = self.parents.get(&index) else {
            return false;
        };
        if self.word(parent) != "{"
            || !matches!(self.lead(index), "{" | ",")
            || !matches!(self.word(index + 1), "}" | ",")
            || self.sites.iter().any(|site| {
                matches!(
                    site.kind,
                    PatternKind::Closure | PatternKind::FunctionParameter
                ) && site.ranges[1].start == parent
            })
        {
            return false;
        }
        let boundary = (0..parent)
            .rev()
            .find(|&i| matches!(self.word(i), ";" | "{" | "}"))
            .map_or(0, |i| i + 1);
        let header = &self.words[boundary..parent];
        let count = |word| header.iter().filter(|value| value.as_str() == word).count();
        let nested = count("(") != count(")") || count("[") != count("]");
        if !nested
            && header
                .iter()
                .any(|word| matches!(word.as_str(), "fn" | "if" | "while" | "match" | "for"))
        {
            return false;
        }
        let owner = self.lead(parent);
        owner == ">"
            || word_syntax::identifier(owner)
                && !matches!(
                    owner,
                    "else" | "loop" | "unsafe" | "async" | "try" | "const" | "return" | "break"
                )
    }
}

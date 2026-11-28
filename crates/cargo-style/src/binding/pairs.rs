//! Read and index the exact pairs that a ticket can explain.

use super::scan;
use crate::rename_admission::BindingRename;
use crate::tokenizer;

pub(crate) enum Pair<'a> {
    Local(BindingRename<'a>),
    NonLocal,
}

#[derive(Clone, Copy)]
pub(crate) enum NameChangePolicy {
    Binding,
    Refactor,
}

pub(crate) enum SingleNameChange<'a> {
    Changed { old: &'a str, new: &'a str },
    Unproved,
}

impl<'a> SingleNameChange<'a> {
    pub(crate) fn read(before: &'a str, after: &'a str, policy: NameChangePolicy) -> Self {
        let old_chars = before.chars().collect::<Vec<_>>();
        let new_chars = after.chars().collect::<Vec<_>>();
        let old_tokens = tokenizer::read_with_comments(&old_chars).tokens;
        let new_tokens = tokenizer::read_with_comments(&new_chars).tokens;
        if old_tokens.len() != new_tokens.len()
            || (matches!(policy, NameChangePolicy::Binding)
                && old_tokens
                    .iter()
                    .chain(&new_tokens)
                    .any(|token| token.value == "::"))
        {
            return Self::Unproved;
        }
        let mut changes = old_tokens
            .iter()
            .zip(&new_tokens)
            .filter(|(old, new)| old.value != new.value || old.kind != new.kind);
        let Some((old, new)) = changes.next() else {
            return Self::Unproved;
        };
        let names = [old.value.as_str(), new.value.as_str()];
        if names.into_iter().any(|name| {
            !scan::local_name(name)
                || (matches!(policy, NameChangePolicy::Binding) && name.starts_with("r#"))
        }) || changes.any(|(old, new)| [old.value.as_str(), new.value.as_str()] != names)
        {
            return Self::Unproved;
        }
        Self::Changed {
            old: source_word(before, old.start, names[0].len()),
            new: source_word(after, new.start, names[1].len()),
        }
    }
}

pub(crate) fn read_pair<'a>(
    kind: &str,
    before: &'a str,
    after: &'a str,
    line: usize,
    text: &'a str,
) -> Pair<'a> {
    if kind != "rename" {
        return Pair::NonLocal;
    }
    let SingleNameChange::Changed { old, new } =
        SingleNameChange::read(before, after, NameChangePolicy::Binding)
    else {
        return Pair::NonLocal;
    };
    Pair::Local(BindingRename {
        line,
        text,
        old,
        new,
    })
}

fn source_word(source: &str, offset: usize, length: usize) -> &str {
    let start = source
        .char_indices()
        .nth(offset)
        .map_or(source.len(), |(start, _)| start);
    &source[start..start + length]
}

pub(super) struct RenamePairs<'a>(Vec<(&'a str, &'a str)>);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PairIndex(usize);

impl<'a> From<&[BindingRename<'a>]> for RenamePairs<'a> {
    fn from(renames: &[BindingRename<'a>]) -> Self {
        let mut pairs = renames
            .iter()
            .map(|rename| (rename.old, rename.new))
            .collect::<Vec<_>>();
        pairs.sort_unstable();
        pairs.dedup();
        Self(pairs)
    }
}

impl RenamePairs<'_> {
    pub(super) fn index(&self, old: &str, new: &str) -> Option<PairIndex> {
        self.0
            .binary_search_by(|pair| pair.cmp(&(old, new)))
            .ok()
            .map(PairIndex)
    }
}

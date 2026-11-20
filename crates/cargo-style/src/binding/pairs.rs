//! Read and index the exact pairs that a ticket can explain.

use super::scan;
use crate::rename_admission::BindingRename;
use crate::tokenizer;

pub(crate) enum Pair<'a> {
    Local(BindingRename<'a>),
    NonLocal,
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
    let old_chars = before.chars().collect::<Vec<_>>();
    let new_chars = after.chars().collect::<Vec<_>>();
    let old_tokens = tokenizer::read_with_comments(&old_chars).tokens;
    let new_tokens = tokenizer::read_with_comments(&new_chars).tokens;
    if old_tokens.len() != new_tokens.len()
        || old_tokens
            .iter()
            .chain(&new_tokens)
            .any(|token| token.value == "::")
    {
        return Pair::NonLocal;
    }
    let mut changes = old_tokens.iter().zip(&new_tokens).filter_map(|(old, new)| {
        (old.value != new.value || old.kind != new.kind).then_some((old, new))
    });
    let Some((old, new)) = changes.next() else {
        return Pair::NonLocal;
    };
    let names = (old.value.as_str(), new.value.as_str());
    if <[&str; 2]>::from(names)
        .into_iter()
        .any(|name| !scan::local_name(name) || name.starts_with("r#"))
        || changes.any(|(old, new)| (old.value.as_str(), new.value.as_str()) != names)
    {
        return Pair::NonLocal;
    }
    let old = source_word(before, old.start, names.0.len());
    let new = source_word(after, new.start, names.1.len());
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

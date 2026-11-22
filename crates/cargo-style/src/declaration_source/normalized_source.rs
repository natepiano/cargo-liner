//! Cache normalized source text and its character offsets.

use std::cell::OnceCell;
use std::ops::Range;

use super::DeclarationFile;
use crate::tokenizer;
use crate::unicode_tables::LINE_BOUNDARIES;

#[derive(Default)]
pub(super) struct NormalizedSource {
    text:    OnceCell<String>,
    offsets: OnceCell<Vec<usize>>,
}

pub(crate) enum NormalizedSlice<'a> {
    Text(&'a str),
    Unavailable,
}

impl DeclarationFile {
    pub(crate) fn normalized_text(&self) -> &str {
        self.normalized.text.get_or_init(|| {
            self.lines()
                .lines()
                .map(|line| {
                    line.trim_end_matches(|character| {
                        tokenizer::in_table(LINE_BOUNDARIES, character)
                    })
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub(crate) fn normalized_slice(&self, range: Range<usize>) -> NormalizedSlice<'_> {
        let text = self.normalized_text();
        let offsets = self.normalized.offsets.get_or_init(|| {
            text.char_indices()
                .map(|(offset, _)| offset)
                .chain([text.len()])
                .collect()
        });
        let (Some(&start), Some(&end)) = (offsets.get(range.start), offsets.get(range.end)) else {
            return NormalizedSlice::Unavailable;
        };
        text.get(start..end)
            .map_or(NormalizedSlice::Unavailable, NormalizedSlice::Text)
    }
}

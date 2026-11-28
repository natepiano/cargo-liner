use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;

use crate::fixes::facade_redirect::SourceLines;
use crate::reporting::AppliedFix;
use crate::reporting::AppliedFixKind;
use crate::rust_syntax::ParsedSources;

struct ChainEdit {
    original: Range<usize>,
    current:  Range<usize>,
    kind:     Option<AppliedFixKind>,
}

impl ChainEdit {
    const fn original_offset(&self, offset: usize) -> usize {
        self.original.end + (offset - self.current.end)
    }
}

/// Map each net edit to both the original source and the current proposal.
#[derive(Default)]
pub(in crate::fixes) struct ChainEdits {
    files: BTreeMap<PathBuf, Vec<ChainEdit>>,
}

impl ChainEdits {
    /// Record edits in the order that the writer applies their byte ranges.
    pub(super) fn record(
        &mut self,
        path: &Path,
        range: Range<usize>,
        replacement_len: usize,
        kind: Option<AppliedFixKind>,
    ) {
        let edits = self.files.entry(path.to_path_buf()).or_default();
        let left = edits.partition_point(|edit| {
            edit.current.end <= range.start && !same_insertion_point(&edit.current, &range)
        });
        let right = left
            + edits[left..].partition_point(|edit| {
                edit.current.start < range.end || same_insertion_point(&edit.current, &range)
            });
        let covered = &edits[left..right];
        let start = covered
            .first()
            .map_or(range.start, |edit| range.start.min(edit.current.start));
        let end = covered
            .last()
            .map_or(range.end, |edit| range.end.max(edit.current.end));
        let preceding = edits[..left].last();
        let original_start = preceding.map_or(start, |edit| edit.original_offset(start));
        let original_end = covered
            .last()
            .or(preceding)
            .map_or(end, |edit| edit.original_offset(end));
        let kind = kind.or_else(|| covered.iter().find_map(|edit| edit.kind));
        let removed_len = range.len();
        let shift = |offset| offset - removed_len + replacement_len;
        for edit in &mut edits[right..] {
            edit.current = shift(edit.current.start)..shift(edit.current.end);
        }
        edits.splice(
            left..right,
            [ChainEdit {
                original: original_start..original_end,
                current: start..shift(end),
                kind,
            }],
        );
    }

    pub(in crate::fixes) fn details(
        &self,
        sources: &ParsedSources,
        proposed: &BTreeMap<PathBuf, String>,
    ) -> Result<Vec<AppliedFix>> {
        let mut details = Vec::new();
        for (path, edits) in &self.files {
            let source = sources.source(path)?;
            let original = source.text();
            let current = &proposed[path];
            if original == current {
                continue;
            }
            let lines = SourceLines::new(original);
            for edit in edits {
                let before = &original[edit.original.clone()];
                let after = &current[edit.current.clone()];
                if let Some(kind) = edit.kind
                    && before != after
                {
                    let offset = edit.original.start + before.len() - before.trim_start().len();
                    details.push(AppliedFix::new(
                        kind,
                        path,
                        lines.line_at(offset),
                        before,
                        after,
                    ));
                }
            }
        }
        Ok(details)
    }
}

fn same_insertion_point(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.is_empty() && right.is_empty() && left.start == right.start
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::ChainEdits;

    #[test]
    fn composing_one_range_keeps_the_other_ranges_in_original_coordinates() {
        let mut edits = ChainEdits::default();
        let path = Path::new("lib.rs");
        edits.record(path, 20..24, 6, None);
        edits.record(path, 4..10, 3, None);
        edits.record(path, 3..8, 0, None);
        let composed = &edits.files[path];
        assert_eq!(composed.len(), 2);
        assert_eq!(composed[0].original, 3..11);
        assert_eq!(composed[0].current, 3..3);
        assert_eq!(composed[1].original, 20..24);
        assert_eq!(composed[1].current, 12..18);
    }
}

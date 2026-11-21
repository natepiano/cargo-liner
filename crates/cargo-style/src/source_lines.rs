//! Read source coordinates and map equal lines across a patch.

use std::ops::RangeInclusive;

use crate::tokenizer;
use crate::unicode_tables::DECIMAL;
use crate::unicode_tables::LINE_BOUNDARIES;

pub(crate) enum SiteLine<'a> {
    Text(&'a str),
    Unavailable,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineContinuity {
    Unchanged,
    Unproved,
}

pub(crate) struct SourceLines {
    text: String,
    ends: Vec<usize>,
}

impl From<String> for SourceLines {
    fn from(text: String) -> Self {
        let mut offset = 0;
        let ends = split_lines(&text)
            .map(|line| {
                offset += line.len();
                offset
            })
            .collect();
        Self { text, ends }
    }
}

impl SourceLines {
    pub(crate) const fn text(&self) -> &str { self.text.as_str() }

    pub(crate) const fn count(&self) -> usize { self.ends.len() }

    pub(crate) fn lines(&self) -> impl Iterator<Item = &str> + '_ {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let line = &self.text[start..end];
            start = end;
            line
        })
    }

    pub(crate) fn slice(&self, lines: RangeInclusive<usize>) -> &str {
        let start = lines.start().saturating_sub(2);
        let end = lines.end().saturating_sub(1);
        let start = if *lines.start() <= 1 {
            0
        } else {
            self.ends.get(start).copied().unwrap_or(self.text.len())
        };
        let end = self.ends.get(end).copied().unwrap_or(self.text.len());
        &self.text[start..end]
    }

    pub(crate) fn line(&self, line: usize) -> SiteLine<'_> {
        if line == 0 || line > self.ends.len() {
            SiteLine::Unavailable
        } else {
            SiteLine::Text(self.slice(line..=line))
        }
    }
}

/// A position in the unchanged intervals of a Myers patch.
#[derive(Clone, Copy)]
pub(crate) enum LineMapping {
    Mapped(usize),
    Changed,
}

/// Read one coordinate from the same intervals as Python `unchanged_lines`.
#[cfg(test)]
fn continuity(diff: &str, line: usize) -> LineContinuity {
    match PatchLineMap::from(diff).at(line) {
        LineMapping::Mapped(mapped) if mapped == line => LineContinuity::Unchanged,
        _ => LineContinuity::Unproved,
    }
}

pub(crate) struct PatchLineMap(Vec<Hunk>);

impl From<&str> for PatchLineMap {
    fn from(diff: &str) -> Self { Self(patch_hunks(diff)) }
}

impl PatchLineMap {
    pub(crate) fn at(&self, line: usize) -> LineMapping {
        let mut old_cursor = 1;
        let mut displacement = 0_i128;
        let mut result = LineMapping::Changed;
        for hunk in &self.0 {
            let boundary = hunk
                .old_start
                .saturating_add(usize::from(hunk.old_count == 0));
            if (old_cursor..boundary).contains(&line) {
                result = translate_line(line, displacement);
            }
            displacement += hunk.added as i128 - hunk.old_count as i128;
            old_cursor = boundary.saturating_add(hunk.old_count);
        }
        if line >= old_cursor {
            translate_line(line, displacement)
        } else {
            result
        }
    }
}

fn translate_line(line: usize, displacement: i128) -> LineMapping {
    usize::try_from(line as i128 + displacement)
        .ok()
        .filter(|&mapped| mapped > 0)
        .map_or(LineMapping::Changed, LineMapping::Mapped)
}

/// Split as Python `str.splitlines(keepends=True)` does.
pub(crate) fn split_lines(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let end = text.char_indices().find_map(|(offset, character)| {
            if !tokenizer::in_table(LINE_BOUNDARIES, character) {
                return None;
            }
            let end = offset + character.len_utf8();
            Some(end + usize::from(character == '\r' && text[end..].starts_with('\n')))
        });
        let (line, rest) = text.split_at(end.unwrap_or(text.len()));
        text = rest;
        Some(line)
    })
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    added:     usize,
}

pub(crate) struct PatchHeader {
    pub(crate) old: PatchRange,
    pub(crate) new: PatchRange,
}

pub(crate) struct PatchRange {
    pub(crate) start: usize,
    pub(crate) count: usize,
}

fn patch_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for line in split_lines(diff) {
        if let Some(header) = patch_header(line) {
            hunks.push(Hunk {
                old_start: header.old.start,
                old_count: header.old.count,
                added:     0,
            });
        } else if let Some(hunk) = hunks.last_mut()
            && line.starts_with('+')
        {
            hunk.added += 1;
        }
    }
    hunks
}

pub(crate) fn patch_header(line: &str) -> Option<PatchHeader> {
    let (old, new) = line.strip_prefix("@@ -")?.split_once(" +")?;
    let (new, _) = new.split_once(" @@")?;
    Some(PatchHeader {
        old: diff_range(old)?,
        new: diff_range(new)?,
    })
}

fn diff_range(range: &str) -> Option<PatchRange> {
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Some(PatchRange {
        start: decimal(start)?,
        count: decimal(count)?,
    })
}

fn decimal(text: &str) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    text.chars().try_fold(0_usize, |value, character| {
        let code = u32::from(character);
        let index = DECIMAL.partition_point(|&(_, end)| end < code);
        let (start, _) = *DECIMAL.get(index)?;
        let digit = usize::try_from(code.checked_sub(start)? % 10).ok()?;
        Some(value.saturating_mul(10).saturating_add(digit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_header_can_replace_an_earlier_equal_interval() {
        let diff = "@@ -7 +7 @@\n-before\n+after\x0b@@ -١,٠ +١,١ @@\x0b+inserted\n";
        assert_eq!(continuity(diff, 3), LineContinuity::Unproved);
    }

    #[test]
    fn a_zero_length_range_counts_all_added_lines() {
        let diff = "--- before\n+++ after\r\n@@ -0,0 +1 @@\r\n+added\r\n@@ -2 +2,0 @@\n-removed\n";
        assert_eq!(continuity(diff, 1), LineContinuity::Unproved);
        assert_eq!(continuity(diff, 2), LineContinuity::Unproved);
        assert_eq!(continuity(diff, 3), LineContinuity::Unchanged);
    }

    #[test]
    fn source_coordinates_keep_python_line_endings() {
        let lines = SourceLines::from("α\r\nβ\x0bγ".to_owned());
        assert!(matches!(lines.line(1), SiteLine::Text("α\r\n")));
        assert!(matches!(lines.line(2), SiteLine::Text("β\x0b")));
        assert!(matches!(lines.line(3), SiteLine::Text("γ")));
        assert!(matches!(lines.line(4), SiteLine::Unavailable));
    }
}

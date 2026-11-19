//! Read source coordinates and map equal lines across a patch.

use std::cell::OnceCell;

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
    ends: OnceCell<Vec<usize>>,
}

impl From<String> for SourceLines {
    fn from(text: String) -> Self {
        Self {
            text,
            ends: OnceCell::new(),
        }
    }
}

impl SourceLines {
    pub(crate) fn text(&self) -> &str { &self.text }

    pub(crate) fn line(&self, line: usize) -> SiteLine<'_> {
        let Some(index) = line.checked_sub(1) else {
            return SiteLine::Unavailable;
        };
        let ends = self.ends.get_or_init(|| {
            split_lines(&self.text)
                .scan(0, |offset, line| {
                    *offset += line.len();
                    Some(*offset)
                })
                .collect()
        });
        let Some(&end) = ends.get(index) else {
            return SiteLine::Unavailable;
        };
        let start = index.checked_sub(1).map_or(0, |previous| ends[previous]);
        SiteLine::Text(&self.text[start..end])
    }
}

/// Read one coordinate from the same intervals as Python `unchanged_lines`.
pub(crate) fn continuity(diff: &str, line: usize) -> LineContinuity {
    let mut old_cursor = 1;
    let mut removed = 0_usize;
    let mut added = 0;
    let mut result = LineContinuity::Unproved;
    for hunk in patch_hunks(diff) {
        let boundary = hunk
            .old_start
            .saturating_add(usize::from(hunk.old_count == 0));
        if (old_cursor..boundary).contains(&line) {
            result = line_continuity(removed, added);
        }
        old_cursor = boundary.saturating_add(hunk.old_count);
        removed = removed.saturating_add(hunk.old_count);
        added += hunk.added;
    }
    if line >= old_cursor {
        line_continuity(removed, added)
    } else {
        result
    }
}

const fn line_continuity(removed: usize, added: usize) -> LineContinuity {
    if removed == added {
        LineContinuity::Unchanged
    } else {
        LineContinuity::Unproved
    }
}

/// Split as Python `str.splitlines(keepends=True)` does.
fn split_lines(mut text: &str) -> impl Iterator<Item = &str> {
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

fn patch_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for line in split_lines(diff) {
        if let Some((old_start, old_count)) = old_range(line) {
            hunks.push(Hunk {
                old_start,
                old_count,
                added: 0,
            });
        } else if let Some(hunk) = hunks.last_mut()
            && line.starts_with('+')
        {
            hunk.added += 1;
        }
    }
    hunks
}

fn old_range(line: &str) -> Option<(usize, usize)> {
    let (old, new) = line.strip_prefix("@@ -")?.split_once(" +")?;
    let (new, _) = new.split_once(" @@")?;
    let _ = diff_range(new)?;
    diff_range(old)
}

fn diff_range(range: &str) -> Option<(usize, usize)> {
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Some((decimal(start)?, decimal(count)?))
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

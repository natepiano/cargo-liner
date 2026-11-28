//! Map byte positions in masked lines joined by LF.

use super::item_scan::Position;

pub(super) struct JoinedSource {
    text:   String,
    starts: Vec<usize>,
}

impl JoinedSource {
    pub(super) fn new(lines: &[&str]) -> Self {
        let starts = lines
            .iter()
            .scan(0, |start, line| {
                let offset = *start;
                *start += line.len() + 1;
                Some(offset)
            })
            .collect();
        Self {
            text: lines.join("\n"),
            starts,
        }
    }

    pub(super) fn text(&self) -> &str { &self.text }

    pub(super) fn position(&self, offset: usize) -> Position {
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        Position {
            line,
            column: offset - self.starts[line],
        }
    }

    pub(super) fn offset(&self, position: Position) -> usize {
        self.starts[position.line] + position.column
    }
}

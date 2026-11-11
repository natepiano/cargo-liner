//! Draw the current resource values at the positions recorded by a cell painting.

use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use tui_pane::label_color;
use tui_pane::text_default;

use super::InvocationId;
use super::RowResourceReading;
use super::TileContent;
use super::TrackedRow;
use super::VisibleRoster;
use super::census;
use super::measurement_text;
use super::memory_label;
use super::summary_memory_rows;

/// The live value columns that draw over a cell painting on each frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ValueColumn {
    Cpu,
    Duration,
    Memory,
}

impl ValueColumn {
    fn reading(
        self,
        row: &TrackedRow,
        reading: Option<&RowResourceReading>,
    ) -> Option<(String, Alignment)> {
        let text = match self {
            Self::Duration => return Some((row.process.duration(), Alignment::Left)),
            Self::Cpu => measurement_text(reading?.resources.cpu, census::scan::cpu_label),
            Self::Memory => measurement_text(reading?.resources.memory, memory_label),
        };
        Some((text, Alignment::Right))
    }
}

/// The source of one live value: a table row's column or a summary memory row.
pub(super) enum PlacedValue {
    Row {
        invocation: InvocationId,
        pid:        u32,
        column:     ValueColumn,
    },
    SummaryMemory(&'static str),
}

/// A painting records where each live value goes and never holds its text.
pub(super) struct PlacedValueText {
    pub(super) value: PlacedValue,
    pub(super) area:  Rect,
}

/// Current readings and row ink for the live values over one cell painting.
pub(super) struct LiveValues<'a, 'row> {
    pub(super) readings: &'a [RowResourceReading],
    pub(super) rows:     &'a [&'row TrackedRow],
    pub(super) summary:  Vec<(&'static str, u64)>,
    pub(super) ground:   Color,
}

impl<'a, 'row> LiveValues<'a, 'row> {
    pub(super) fn of(
        readings: &'a [RowResourceReading],
        rows: &'a [&'row TrackedRow],
        roster: &VisibleRoster,
        content: &TileContent,
        ground: Color,
    ) -> Self {
        Self {
            readings,
            rows,
            summary: match content {
                TileContent::Summary => summary_memory_rows(roster).collect(),
                TileContent::Group(_) | TileContent::Empty(_) => Vec::new(),
            },
            ground,
        }
    }
}

/// Fill each value's whole area, then draw its current text in its row's current ink.
pub(super) fn draw_live_values(
    buffer: &mut Buffer,
    values: &[PlacedValueText],
    current: &LiveValues<'_, '_>,
) {
    for placed in values {
        let area = placed.area.intersection(buffer.area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buffer[(x, y)].set_symbol(" ");
            }
        }
        let line = match &placed.value {
            PlacedValue::Row {
                invocation,
                pid,
                column,
            } => {
                let Some(row) = current.rows.iter().find(|row| {
                    &row.process.invocation_id == invocation && row.process.pid == *pid
                }) else {
                    continue;
                };
                let reading = current
                    .readings
                    .iter()
                    .find(|reading| &reading.invocation == invocation && reading.pid == *pid);
                let Some((text, alignment)) = column.reading(row, reading) else {
                    continue;
                };
                Line::styled(
                    text,
                    Style::default().fg(row.ink().color(label_color(), current.ground)),
                )
                .alignment(alignment)
            },
            PlacedValue::SummaryMemory(label) => {
                let Some((_, bytes)) = current.summary.iter().find(|(known, _)| known == label)
                else {
                    continue;
                };
                Line::styled(memory_label(*bytes), Style::default().fg(text_default()))
            },
        };
        line.render(area, buffer);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::buffer::Cell;

    use super::*;
    use crate::census::scan::OriginStart;
    use crate::render::tests;

    #[test]
    fn shorter_durations_erase_old_characters_and_keep_the_row_background() {
        let area = Rect::new(3, 5, 8, 1);
        let ground = Color::Indexed(16);
        let mut process = tests::invocation(4100, &["build"]);
        process.origin_start = OriginStart::Process {
            epoch:   1_790_000_000,
            elapsed: Duration::from_secs(3600),
        };
        let first = TrackedRow::from(process.clone());
        process.origin_start = OriginStart::Process {
            epoch:   1_790_000_000,
            elapsed: Duration::from_secs(59),
        };
        let shorter = TrackedRow::from(process);
        let values = [PlacedValueText {
            value: PlacedValue::Row {
                invocation: first.process.invocation_id.clone(),
                pid:        first.process.pid,
                column:     ValueColumn::Duration,
            },
            area,
        }];
        let mut actual = Buffer::empty(area);
        actual.set_style(area, Style::default().bg(ground));
        let mut expected = actual.clone();
        draw_live_values(
            &mut actual,
            &values,
            &LiveValues {
                readings: &[],
                rows: &[&first],
                summary: Vec::new(),
                ground,
            },
        );
        assert_eq!(
            actual.content.iter().map(Cell::symbol).collect::<String>(),
            "01:00:00"
        );
        draw_live_values(
            &mut actual,
            &values,
            &LiveValues {
                readings: &[],
                rows: &[&shorter],
                summary: Vec::new(),
                ground,
            },
        );
        draw_live_values(
            &mut expected,
            &values,
            &LiveValues {
                readings: &[],
                rows: &[&shorter],
                summary: Vec::new(),
                ground,
            },
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual.content.iter().map(Cell::symbol).collect::<String>(),
            "00:59   "
        );
        assert!(actual.content.iter().all(|cell| cell.bg == ground));
    }
}

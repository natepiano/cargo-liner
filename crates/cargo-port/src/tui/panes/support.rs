use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::PaneFocusState;
use tui_pane::VisualRowSpan;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::package;

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in text.split_whitespace() {
        let word_width = word.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
        if current_width > 0 && current_width + 1 + word_width > max_width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        if current_width > 0 {
            current.push(' ');
            current_width += 1;
        }
        if word_width > max_width {
            let wrapped = hard_wrap(word, max_width);
            for (i, part) in wrapped.into_iter().enumerate() {
                if i == 0 {
                    current.push_str(&part);
                    current_width += part.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
                } else {
                    lines.push(current);
                    current = part;
                    current_width = current.chars().map(|c| c.width().unwrap_or(0)).sum();
                }
            }
        } else {
            current.push_str(word);
            current_width += word_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(super) fn hard_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width + width > max_width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The rows that `ratatui::widgets::Table` can draw, for rows of one height.
///
/// The table starts at `offset`. It moves up to a `selected` row above that,
/// and down until a `selected` row below the area shows. It also draws one
/// partial row. A row outside the range needs only its height, so a caller
/// builds those rows empty. `area_height` can include the header: a taller
/// area gives a wider range.
pub(super) fn table_draw_window(
    len: usize,
    offset: usize,
    selected: usize,
    row_height: u16,
    area_height: u16,
) -> Range<usize> {
    if row_height == 0 {
        return 0..len;
    }
    let last = len.saturating_sub(1);
    let selected = selected.min(last);
    let offset = offset.min(last);
    let fit = usize::from(area_height / row_height);
    let start = if selected >= offset + fit {
        (selected + 1).saturating_sub(fit)
    } else {
        offset
    }
    .min(selected);
    let end = (start + fit + 1).max(selected + 2).min(len);
    start..end
}

/// Inner width where fields can place their values on a separate row.
pub(super) const NARROW_FIELD_INNER_WIDTH: usize = 15;

pub(super) fn field_value_lines(
    prefix: String,
    value: &str,
    width: usize,
    label_style: Style,
    value_style: Style,
    separate_value: bool,
) -> Vec<Line<'static>> {
    let width = width.saturating_sub(1);
    let available = width.saturating_sub(prefix.width());
    if separate_value && value.width() > available {
        return vec![
            Line::from(Span::styled(prefix.trim_end().to_string(), label_style)),
            Line::from(Span::styled(
                format!(
                    " {}",
                    super::super::render::truncate_with_ellipsis(
                        value,
                        width.saturating_sub(1),
                        "…"
                    )
                ),
                value_style,
            )),
        ];
    }
    vec![Line::from(vec![
        Span::styled(prefix, label_style),
        Span::styled(
            super::super::render::truncate_with_ellipsis(value, available, "…"),
            value_style,
        ),
    ])]
}

/// Page positions for source lines, independent of selectable row spans.
pub(super) struct DetailPages {
    positions:         Vec<usize>,
    pub scroll_offset: u16,
}

impl DetailPages {
    pub(super) fn line_y(&self, source_y: usize) -> usize {
        self.positions.get(source_y).copied().unwrap_or(source_y)
    }

    pub(super) fn place_span(&self, span: &mut VisualRowSpan) {
        let last = self.line_y(span.start_y + span.height.saturating_sub(1));
        span.start_y = self.line_y(span.start_y);
        span.height = last.saturating_sub(span.start_y) + 1;
    }
}

/// Keep fields and header groups together when they fit on a page.
pub(super) fn paginate_detail_lines(
    lines: &mut Vec<Line<'static>>,
    fields: &[VisualRowSpan],
    headers: &[VisualRowSpan],
    focused_line: usize,
    height: u16,
    focus: PaneFocusState,
) -> DetailPages {
    paginate_detail_sections(
        lines,
        fields,
        &DetailSectionHeaders {
            groups:   headers,
            repeated: &[],
        },
        focused_line,
        height,
        focus,
    )
}

/// Headers used for grouping and for page continuations.
pub(super) struct DetailSectionHeaders<'a> {
    pub groups:   &'a [VisualRowSpan],
    pub repeated: &'a [VisualRowSpan],
}

pub(super) fn paginate_detail_sections(
    lines: &mut Vec<Line<'static>>,
    fields: &[VisualRowSpan],
    headers: &DetailSectionHeaders<'_>,
    focused_line: usize,
    height: u16,
    focus: PaneFocusState,
) -> DetailPages {
    let height = usize::from(height).max(1);
    let mut groups: Vec<_> = fields.iter().chain(headers.groups).copied().collect();
    groups.sort_by_key(|span| span.start_y);
    let mut merged: Vec<VisualRowSpan> = Vec::new();
    for group in groups {
        if let Some(previous) = merged.last_mut()
            && group.start_y < previous.start_y + previous.height
        {
            previous.height = previous
                .height
                .max(group.start_y + group.height - previous.start_y);
        } else {
            merged.push(group);
        }
    }
    let source = std::mem::take(lines);
    let mut positions = Vec::with_capacity(source.len());
    let mut groups = merged.iter().peekable();
    for (source_y, line) in source.iter().enumerate() {
        if let Some(group) = groups.peek()
            && group.start_y == source_y
        {
            let used = lines.len() % height;
            if used > 0 && group.height > height - used {
                lines.resize(lines.len() + height - used, Line::default());
            }
            groups.next();
        }
        repeat_section_header(lines, &source, headers.repeated, source_y, height);
        positions.push(lines.len());
        lines.push(line.clone());
    }
    let focused_y = positions.get(focused_line).copied().unwrap_or(0);
    let offset = usize::from(package::detail_column_scroll_offset(
        focus,
        focused_y,
        u16::try_from(height).unwrap_or(u16::MAX),
        lines.len(),
    ));
    let offset = offset.div_ceil(height) * height;
    DetailPages {
        positions,
        scroll_offset: u16::try_from(offset).unwrap_or(u16::MAX),
    }
}

fn repeat_section_header(
    lines: &mut Vec<Line<'static>>,
    source: &[Line<'static>],
    headers: &[VisualRowSpan],
    source_y: usize,
    height: usize,
) {
    if !lines.len().is_multiple_of(height) {
        return;
    }
    if let Some(header) = headers
        .iter()
        .rev()
        .find(|header| header.start_y <= source_y)
        && header.start_y + header.height <= source_y
        && header.height < height
    {
        lines.extend_from_slice(&source[header.start_y..header.start_y + header.height]);
    }
}

/// Join each unselectable header to its first selectable row.
pub(super) fn detail_header_groups(
    fields: &[VisualRowSpan],
    starts: impl IntoIterator<Item = usize>,
) -> Vec<VisualRowSpan> {
    starts
        .into_iter()
        .filter_map(|start_y| {
            fields
                .iter()
                .find(|span| span.start_y >= start_y)
                .map(|row| VisualRowSpan {
                    start_y,
                    height: row.start_y + row.height - start_y,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Constraint;
    use ratatui::widgets::Row;
    use ratatui::widgets::Table;
    use ratatui::widgets::TableState;

    use super::table_draw_window;

    fn draw(
        rows: Vec<Row<'static>>,
        offset: usize,
        selected: usize,
        height: u16,
    ) -> anyhow::Result<(String, usize)> {
        let backend = TestBackend::new(6, height);
        let mut terminal = Terminal::new(backend)?;
        let mut state = TableState::default().with_selected(Some(selected));
        *state.offset_mut() = offset;
        let table = Table::new(rows, [Constraint::Length(6)]).header(Row::new(["head"]));
        terminal.draw(|frame| frame.render_stateful_widget(table, frame.area(), &mut state))?;
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        Ok((text, state.offset()))
    }

    #[test]
    fn empty_rows_outside_the_draw_window_draw_the_same_table() -> anyhow::Result<()> {
        let len = 8;
        for row_height in 1..=2 {
            for height in 1..=7 {
                for offset in 0..len + 2 {
                    for selected in 0..len + 2 {
                        let full = (0..len)
                            .map(|index| Row::new([format!("r{index}")]).height(row_height))
                            .collect();
                        let window = table_draw_window(len, offset, selected, row_height, height);
                        let fit = usize::from(height / row_height);
                        assert!(
                            window.len() <= fit + 2,
                            "row_height={row_height} height={height} offset={offset} selected={selected} window={window:?}"
                        );
                        let windowed = (0..len)
                            .map(|index| {
                                if window.contains(&index) {
                                    Row::new([format!("r{index}")]).height(row_height)
                                } else {
                                    Row::default().height(row_height)
                                }
                            })
                            .collect();
                        assert_eq!(
                            draw(full, offset, selected, height)?,
                            draw(windowed, offset, selected, height)?,
                            "row_height={row_height} height={height} offset={offset} selected={selected}"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

//! Targets pane render bodies.
//!
//! `render_targets_panel` and `render_empty_targets_panel` live
//! alongside `TargetsPane`. They are free functions (no `Pane`
//! trait impl) because the body touches App shell state during
//! render — `pane_focus_state` plus the typed
//! `panes_mut().targets_mut().viewport` accessors.

use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::render_overflow_affordance;

use super::TargetsData;
use super::package::RenderStyles;
use super::spec::PaneId;
use crate::tui::app::App;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneTitleCount;
use crate::tui::pane::PaneTitleGroup;
use crate::tui::panes;
use crate::tui::render;

pub fn render_empty_targets_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    app.panes.targets.viewport.clear_surface();
    let empty_targets = pane::empty_pane_block(" No Targets ");
    frame.render_widget(empty_targets, area);
}

pub fn render_targets_panel(
    frame: &mut Frame,
    app: &mut App,
    data: &TargetsData,
    styles: &RenderStyles,
    area: Rect,
) {
    let bin_count: usize = usize::from(data.primary_binary.is_some());
    let ex_count: usize = data.examples.iter().map(|group| group.names.len()).sum();
    let bench_count = data.benches.len();

    let focus = app.pane_focus_state(PaneId::Targets);
    let cursor = app.panes.targets.viewport.pos();

    let targets_title = {
        let focused_cursor = matches!(focus, PaneFocusState::Active).then_some(cursor);
        let section_cursor = |section_start: usize, section_len: usize| {
            focused_cursor
                .filter(|cursor| *cursor >= section_start && *cursor < section_start + section_len)
                .map(|cursor| cursor - section_start)
        };
        let mut groups = Vec::new();
        if bin_count > 0 {
            groups.push(PaneTitleGroup {
                label:  "Binary".into(),
                len:    bin_count,
                cursor: section_cursor(0, bin_count),
            });
        }
        if ex_count > 0 {
            groups.push(PaneTitleGroup {
                label:  "Examples".into(),
                len:    ex_count,
                cursor: section_cursor(bin_count, ex_count),
            });
        }
        if bench_count > 0 {
            groups.push(PaneTitleGroup {
                label:  "Benches".into(),
                len:    bench_count,
                cursor: section_cursor(bin_count + ex_count, bench_count),
            });
        }
        pane::prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))
    };

    let targets_block = styles
        .chrome
        .block(targets_title, matches!(focus, PaneFocusState::Active));

    let entries = panes::build_target_list_from_data(data);
    app.panes.targets.viewport.set_len(entries.len());
    let content_inner = targets_block.inner(area);
    app.panes.targets.viewport.set_content_area(content_inner);
    app.panes
        .targets
        .viewport
        .set_viewport_rows(usize::from(content_inner.height));

    let kind_col_width = panes::RunTargetKind::padded_label_width();
    let col_spacing: usize = 1;
    let leading_pad: usize = 1;
    let name_max_width =
        (content_inner.width as usize).saturating_sub(kind_col_width + col_spacing + leading_pad);

    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(row_index, entry)| {
            let display =
                render::truncate_with_ellipsis(&entry.display_name, name_max_width, "\u{2026}");
            Row::new(vec![
                Cell::from(format!(" {display}")),
                Cell::from(
                    Line::from(format!("{} ", entry.kind.label())).alignment(Alignment::Right),
                )
                .style(Style::default().fg(entry.kind.color())),
            ])
            .style(
                app.panes
                    .targets
                    .viewport
                    .selection_state(row_index, focus)
                    .overlay_style(),
            )
        })
        .collect();

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(kind_col_width).unwrap_or(u16::MAX)),
    ];
    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.panes
        .targets
        .viewport
        .set_scroll_offset(table_state.offset());
    render_overflow_affordance(
        frame,
        area,
        app.panes.targets.viewport.overflow(),
        Style::default().fg(LABEL_COLOR),
    );
}

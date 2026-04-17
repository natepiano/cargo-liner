use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use super::PaneId;
use super::package::RenderStyles;
use crate::project::LangEntry;
use crate::project::LanguageStats;
use crate::tui::app::App;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneTitleCount;
use crate::tui::render;

/// Fixed numeric column width for language stats.
const LANG_NUM_COL: u16 = 8;

/// Column constraints for the language stats table.
const fn lang_table_widths() -> [Constraint; 7] {
    [
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(LANG_NUM_COL),
        Constraint::Length(LANG_NUM_COL),
        Constraint::Length(LANG_NUM_COL),
        Constraint::Length(LANG_NUM_COL),
        Constraint::Length(LANG_NUM_COL),
    ]
}

fn lang_header_row() -> Row<'static> {
    let style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);
    Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        Cell::from(ratatui::text::Line::from("files").alignment(Alignment::Right)).style(style),
        Cell::from(ratatui::text::Line::from("code").alignment(Alignment::Right)).style(style),
        Cell::from(ratatui::text::Line::from("comments").alignment(Alignment::Right)).style(style),
        Cell::from(ratatui::text::Line::from("blanks").alignment(Alignment::Right)).style(style),
        Cell::from(ratatui::text::Line::from("total").alignment(Alignment::Right)).style(style),
    ])
}

fn lang_footer_row(stats: &LanguageStats) -> Row<'static> {
    let num_bold = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);
    let dim_bold = Style::default()
        .fg(LABEL_COLOR)
        .add_modifier(Modifier::BOLD);
    let total_files: usize = stats.entries.iter().map(|e| e.files).sum();
    let total_code: usize = stats.entries.iter().map(|e| e.code).sum();
    let total_comments: usize = stats.entries.iter().map(|e| e.comments).sum();
    let total_blanks: usize = stats.entries.iter().map(|e| e.blanks).sum();
    let grand_total = total_code + total_comments + total_blanks;
    Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        Cell::from(ratatui::text::Line::from(total_files.to_string()).alignment(Alignment::Right))
            .style(num_bold),
        Cell::from(ratatui::text::Line::from(total_code.to_string()).alignment(Alignment::Right))
            .style(num_bold),
        Cell::from(
            ratatui::text::Line::from(total_comments.to_string()).alignment(Alignment::Right),
        )
        .style(dim_bold),
        Cell::from(ratatui::text::Line::from(total_blanks.to_string()).alignment(Alignment::Right))
            .style(dim_bold),
        Cell::from(ratatui::text::Line::from(grand_total.to_string()).alignment(Alignment::Right))
            .style(num_bold),
    ])
}

fn lang_entry_row(entry: &LangEntry, name_width: usize) -> Row<'static> {
    let icon = crate::project::language_icon(&entry.language);
    let name = render::truncate_with_ellipsis(&entry.language, name_width, "\u{2026}");
    let total = entry.code + entry.comments + entry.blanks;
    let num_style = Style::default().fg(TITLE_COLOR);
    let dim_style = Style::default().fg(LABEL_COLOR);
    Row::new(vec![
        Cell::from(format!(" {icon}")),
        Cell::from(name).style(dim_style),
        Cell::from(ratatui::text::Line::from(entry.files.to_string()).alignment(Alignment::Right))
            .style(num_style),
        Cell::from(ratatui::text::Line::from(entry.code.to_string()).alignment(Alignment::Right))
            .style(num_style),
        Cell::from(
            ratatui::text::Line::from(entry.comments.to_string()).alignment(Alignment::Right),
        )
        .style(dim_style),
        Cell::from(ratatui::text::Line::from(entry.blanks.to_string()).alignment(Alignment::Right))
            .style(dim_style),
        Cell::from(ratatui::text::Line::from(total.to_string()).alignment(Alignment::Right))
            .style(num_style),
    ])
}

fn build_lang_rows(
    app: &App,
    stats: &LanguageStats,
    name_width: usize,
    focus: PaneFocusState,
) -> Vec<Row<'static>> {
    stats
        .entries
        .iter()
        .enumerate()
        .map(|(row_index, entry)| {
            lang_entry_row(entry, name_width).style(
                app.pane_manager()
                    .pane(PaneId::Lang)
                    .selection_state(row_index, focus)
                    .overlay_style(),
            )
        })
        .collect()
}

fn render_lang_table(
    frame: &mut Frame,
    app: &mut App,
    rows: Vec<Row<'static>>,
    widths: [Constraint; 7],
    body_area: Rect,
) {
    let cursor = app.pane_manager().pane(PaneId::Lang).pos();
    let table = Table::new(rows, widths)
        .column_spacing(1)
        .row_highlight_style(Style::default());
    let mut table_state = TableState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(table, body_area, &mut table_state);
    app.pane_manager_mut()
        .pane_mut(PaneId::Lang)
        .set_scroll_offset(table_state.offset());
}

/// Render the Languages panel as a standalone pane.
pub fn render_lang_panel_standalone(
    frame: &mut Frame,
    app: &mut App,
    styles: &RenderStyles,
    area: Rect,
) {
    let lang_stats = app
        .projects()
        .at_path(
            app.selected_project_path()
                .unwrap_or_else(|| std::path::Path::new("")),
        )
        .and_then(|p| p.language_stats.as_ref())
        .cloned();

    let lang_count = lang_stats.as_ref().map_or(0, |s| s.entries.len());
    let lang_focus = app.pane_focus_state(PaneId::Lang);
    let cursor = matches!(lang_focus, crate::tui::pane::PaneFocusState::Active)
        .then(|| app.pane_manager().pane(PaneId::Lang).pos());
    let title = pane::pane_title(
        "Languages",
        &PaneTitleCount::Single {
            len: lang_count,
            cursor,
        },
    );
    let block = styles.chrome.block(title, app.is_focused(PaneId::Lang));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(stats) = lang_stats else {
        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .clear_surface();
        frame.render_widget(Paragraph::new("  Scanning..."), inner);
        return;
    };

    if stats.entries.is_empty() {
        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .clear_surface();
        frame.render_widget(Paragraph::new("  No source files detected"), inner);
        return;
    }

    if inner.height < 2 {
        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .clear_surface();
        return;
    }

    let widths = lang_table_widths();
    let entry_count = stats.entries.len();

    let fixed_cols = 3 + 5 * usize::from(LANG_NUM_COL) + 6;
    let name_width = usize::from(inner.width).saturating_sub(fixed_cols);

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Table::new([lang_header_row()], widths).column_spacing(1),
        header_area,
    );

    let content_below_header = inner.height.saturating_sub(1);
    let rows_needed = u16::try_from(entry_count + 1).unwrap_or(u16::MAX);
    let pin_footer = rows_needed > content_below_header;

    let mut rows = build_lang_rows(app, &stats, name_width, lang_focus);

    if pin_footer {
        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        frame.render_widget(
            Table::new([lang_footer_row(&stats)], widths).column_spacing(1),
            footer_area,
        );
        let body_height = inner.height.saturating_sub(2);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .set_len(rows.len());
        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .set_content_area(body_area);
        render_lang_table(frame, app, rows, widths, body_area);
    } else {
        rows.push(lang_footer_row(&stats));
        let body_height = inner.height.saturating_sub(1);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .set_len(entry_count);
        app.pane_manager_mut()
            .pane_mut(PaneId::Lang)
            .set_content_area(body_area);
        render_lang_table(frame, app, rows, widths, body_area);
    }
}

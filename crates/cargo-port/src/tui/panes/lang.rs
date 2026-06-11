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
use tui_pane::PaneFocusState;
use tui_pane::PaneTitleCount;
use tui_pane::Viewport;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;
use tui_pane::title_color;

use super::constants::LANG_NUM_COL;
use super::package::RenderStyles;
use super::pane_impls::LangPane;
use crate::project::LangEntry;
use crate::project::LanguageStats;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::render;
use crate::tui::theme_roles;

/// Map a tokei language name to a 2-char icon for the Lang column.
pub(super) fn language_icon(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "rust" => "\u{1f980}",                                                 // 🦀
        "c" | "c++" | "c header" | "c++ header" | "c++ module" => "\u{1f30a}", // 🌊
        "java" => "\u{2615}",                                                  // ☕
        "go" => "Go",
        "python" => "\u{1f40d}", // 🐍
        "javascript" | "jsx" => "JS",
        "typescript" | "tsx" => "TS",
        "markdown" => "M\u{2193}", // M↓
        "shell" | "bash" | "zsh" | "fish" => "$_",
        "liquid" => "\u{1f4a7}",      // 💧
        "toml" => "\u{2699}\u{fe0f}", // ⚙️
        "json" => "{}",
        "html" => "\u{1f310}",       // 🌐
        "plain text" => "\u{1f4c4}", // 📄
        "xml" => "<>",
        "glsl" | "webgpu shader language" => "\u{1f53a}", // 🔺 (shading languages)
        "svg" => "\u{1f4d0}",                             // 📐
        "yaml" => "Y:",
        "bitbake" => "\u{1f35e}",          // 🍞
        "cmake" => "\u{1f528}",            // 🔨
        "makefile" => "\u{1f6e0}\u{fe0f}", // 🛠️
        "autoconf" => "\u{1f527}",         // 🔧
        "asciidoc" => "A\u{2193}",         // A↓
        "batch" => "C:",
        // tokei reports RON as "Rusty Object Notation"; abbreviate to RON.
        "rusty object notation" => "RON",
        _ => "  ",
    }
}

/// Column constraints for the language stats table.
const fn lang_table_widths() -> [Constraint; 7] {
    [
        // Icon column: 1-space inset + up to a 3-char text icon (e.g. "RON").
        Constraint::Length(4),
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
        .fg(theme_roles::column_header_color())
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
        .fg(title_color())
        .add_modifier(Modifier::BOLD);
    let data_bold = Style::default()
        .fg(text_default())
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
            .style(data_bold),
        Cell::from(
            ratatui::text::Line::from(total_comments.to_string()).alignment(Alignment::Right),
        )
        .style(data_bold),
        Cell::from(ratatui::text::Line::from(total_blanks.to_string()).alignment(Alignment::Right))
            .style(data_bold),
        Cell::from(ratatui::text::Line::from(grand_total.to_string()).alignment(Alignment::Right))
            .style(num_bold),
    ])
}

fn lang_entry_row(entry: &LangEntry, name_width: usize, indent: usize) -> Row<'static> {
    let icon = if indent == 0 {
        format!(" {}", language_icon(&entry.language))
    } else {
        String::new()
    };
    let label = format!("{}{}", " ".repeat(indent), entry.language);
    let name = render::truncate_with_ellipsis(&label, name_width, "\u{2026}");
    let total = entry.code + entry.comments + entry.blanks;
    let is_subtotal = indent > 0;
    let num_style = if is_subtotal {
        Style::default().fg(theme_roles::language_subtotal_color())
    } else {
        Style::default().fg(title_color())
    };
    let dim_style = if is_subtotal {
        Style::default().fg(theme_roles::language_subtotal_color())
    } else {
        Style::default().fg(label_color())
    };
    let data_style = if is_subtotal {
        num_style
    } else {
        Style::default().fg(text_default())
    };
    Row::new(vec![
        Cell::from(icon),
        Cell::from(name).style(dim_style),
        Cell::from(ratatui::text::Line::from(entry.files.to_string()).alignment(Alignment::Right))
            .style(num_style),
        Cell::from(ratatui::text::Line::from(entry.code.to_string()).alignment(Alignment::Right))
            .style(data_style),
        Cell::from(
            ratatui::text::Line::from(entry.comments.to_string()).alignment(Alignment::Right),
        )
        .style(data_style),
        Cell::from(ratatui::text::Line::from(entry.blanks.to_string()).alignment(Alignment::Right))
            .style(data_style),
        Cell::from(ratatui::text::Line::from(total.to_string()).alignment(Alignment::Right))
            .style(num_style),
    ])
}

struct LangRenderEntry<'a> {
    entry:  &'a LangEntry,
    indent: usize,
}

fn flatten_lang_entries(stats: &LanguageStats) -> Vec<LangRenderEntry<'_>> {
    let mut rows = Vec::new();
    for entry in &stats.entries {
        rows.push(LangRenderEntry { entry, indent: 0 });
        rows.extend(entry.children.iter().map(|child| LangRenderEntry {
            entry:  child,
            indent: 2,
        }));
    }
    rows
}

fn build_lang_rows(
    viewport: &Viewport,
    entries: &[LangRenderEntry<'_>],
    name_width: usize,
    focus: PaneFocusState,
) -> Vec<Row<'static>> {
    entries
        .iter()
        .enumerate()
        .map(|(row_index, render_entry)| {
            lang_entry_row(render_entry.entry, name_width, render_entry.indent)
                .style(tui_pane::selection_state(viewport, row_index, focus).overlay_style())
        })
        .collect()
}

fn render_lang_table(
    frame: &mut Frame,
    pane: &mut LangPane,
    rows: Vec<Row<'static>>,
    widths: [Constraint; 7],
    body_area: Rect,
) {
    let cursor = pane.viewport.pos();
    let table = Table::new(rows, widths)
        .column_spacing(1)
        .row_highlight_style(Style::default());
    let mut table_state = TableState::default().with_selected(Some(cursor));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(table, body_area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
}

/// Body of `LangPane::render`. Same pattern as
/// `cpu::render_cpu_pane_body`: typed parameters via `ctx`.
pub(super) fn render_lang_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut LangPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let focus_state = pane.focus.state;
    let is_focused = pane.focus.is_focused;
    let PaneRenderCtx {
        project_list: projects,
        selected_project_path,
        ..
    } = ctx;

    let lang_stats = projects
        .at_path(selected_project_path.unwrap_or_else(|| std::path::Path::new("")))
        .and_then(|p| p.language_stats.as_ref())
        .cloned();

    let lang_count = lang_stats
        .as_ref()
        .map_or(0, |s| flatten_lang_entries(s).len());
    let cursor = matches!(focus_state, PaneFocusState::Active).then(|| pane.viewport.pos());
    let title = tui_pane::pane_title(
        "Languages",
        &PaneTitleCount::Single {
            len: lang_count,
            cursor,
        },
    );
    let block = styles.chrome.block(title, is_focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(stats) = lang_stats else {
        pane.viewport.clear_surface();
        frame.render_widget(Paragraph::new("  Scanning..."), inner);
        return;
    };

    if stats.entries.is_empty() {
        pane.viewport.clear_surface();
        frame.render_widget(Paragraph::new("  No source files detected"), inner);
        return;
    }

    if inner.height < 2 {
        pane.viewport.clear_surface();
        return;
    }

    let widths = lang_table_widths();
    let entries = flatten_lang_entries(&stats);
    let entry_count = entries.len();

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

    let mut rows = build_lang_rows(&pane.viewport, &entries, name_width, focus_state);

    if pin_footer {
        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        frame.render_widget(
            Table::new([lang_footer_row(&stats)], widths).column_spacing(1),
            footer_area,
        );
        let body_height = inner.height.saturating_sub(2);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        let viewport = &mut pane.viewport;
        viewport.set_len(rows.len());
        viewport.set_content_area(body_area);
        viewport.set_viewport_rows(usize::from(body_area.height));
        render_lang_table(frame, pane, rows, widths, body_area);
    } else {
        rows.push(lang_footer_row(&stats));
        let body_height = inner.height.saturating_sub(1);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        let viewport = &mut pane.viewport;
        viewport.set_len(entry_count);
        viewport.set_content_area(body_area);
        viewport.set_viewport_rows(usize::from(body_area.height));
        render_lang_table(frame, pane, rows, widths, body_area);
    }

    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
}

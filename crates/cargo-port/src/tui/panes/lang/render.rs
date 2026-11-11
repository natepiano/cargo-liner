use std::path::Path;

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
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneTitleCount;
use tui_pane::Viewport;
use tui_pane::label_color;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::constants::LANG_COLUMN_GAP;
use super::constants::LANG_HEADERS;
use super::constants::LANG_ICON_WIDTH;
use super::constants::LANG_RIGHT_MARGIN;
use super::constants::LANG_SUBTOTAL_MARK;
use super::pane::LangPane;
use crate::project::LangEntry;
use crate::project::LanguageStats;
use crate::tui::panes::ProjectPresentation;
use crate::tui::project_list::ProjectPathLookup::Found;
use crate::tui::project_list::ProjectPathLookup::StaleRoute;
use crate::tui::project_list::ProjectPathLookup::UnknownPath;
use crate::tui::project_list_state::ProjectList;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::theme_roles;

/// Map a tokei language name to an icon for the Lang column.
pub fn language_icon(language: &str) -> &'static str {
    // Each trailing comment shows the glyph rendered by its escape in `language_icon`.
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
        // `language_icon` maps tokei's "Rusty Object Notation" name to the abbreviated RON label.
        "rusty object notation" => "RON",
        _ => "  ",
    }
}

/// The pane width that draws every language name whole.
pub(in crate::tui::panes) fn preferred_width(stats: &LanguageStats) -> u16 {
    let name_width = stats
        .entries
        .iter()
        .flat_map(|entry| {
            std::iter::once(entry.language.width()).chain(
                entry
                    .children
                    .iter()
                    .map(|child| LANG_SUBTOTAL_MARK.width() + child.language.width()),
            )
        })
        .max()
        .unwrap_or(0);
    let width = fixed_columns_width(&lang_table_widths(stats))
        + name_width
        + usize::from(LANG_RIGHT_MARGIN)
        + tui_pane::BLOCK_BORDER_WIDTH;
    u16::try_from(width)
        .unwrap_or(u16::MAX)
        .max(minimum_width())
}

/// The inner rows of the Languages pane for `language_rows` table rows: the
/// header, the rows and the footer, or one row for a message.
pub(in crate::tui::panes) fn inner_rows(language_rows: usize) -> u16 {
    if language_rows == 0 {
        return 1;
    }
    u16::try_from(language_rows)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
}

/// The language stats of the selected project, when the scan has them.
pub(in crate::tui) fn selected_language_stats<'a>(
    projects: &'a ProjectList,
    selected: Option<&Path>,
) -> Option<&'a LanguageStats> {
    match projects.at_path(selected.unwrap_or_else(|| Path::new(""))) {
        Found(project) => project.language_stats.as_ref(),
        UnknownPath | StaleRoute => None,
    }
}

/// Cells of the fixed-width columns and of every gap between columns.
fn fixed_columns_width(widths: &[Constraint; 7]) -> usize {
    widths
        .iter()
        .map(|constraint| match constraint {
            Constraint::Length(width) => usize::from(*width),
            _ => 0,
        })
        .sum::<usize>()
        + (widths.len() - 1) * usize::from(LANG_COLUMN_GAP)
}

pub(in crate::tui::panes) fn minimum_width() -> u16 {
    let numeric = LANG_HEADERS
        .iter()
        .map(|header| header.width())
        .sum::<usize>();
    u16::try_from(
        numeric
            + "Languages".width()
            + usize::from(LANG_ICON_WIDTH)
            + LANG_HEADERS.len()
            + 1
            + tui_pane::BLOCK_BORDER_WIDTH,
    )
    .unwrap_or(u16::MAX)
}

fn parent_position(stats: &LanguageStats, row: usize) -> Option<usize> {
    let mut start = 0;
    for (index, entry) in stats.entries.iter().enumerate() {
        start += 1 + entry.children.len();
        if row < start {
            return Some(index);
        }
    }
    None
}

fn lang_totals(stats: &LanguageStats) -> [usize; 5] {
    let files = stats.entries.iter().map(|entry| entry.files).sum();
    let code = stats.entries.iter().map(|entry| entry.code).sum();
    let comments = stats.entries.iter().map(|entry| entry.comments).sum();
    let blanks = stats.entries.iter().map(|entry| entry.blanks).sum();
    [files, code, comments, blanks, code + comments + blanks]
}

fn lang_table_widths(stats: &LanguageStats) -> [Constraint; 7] {
    let totals = lang_totals(stats);
    let icon_width = stats
        .entries
        .iter()
        .map(|entry| language_icon(&entry.language).width())
        .max()
        .unwrap_or_else(|| language_icon("").width())
        + 1;
    std::array::from_fn(|index| match index {
        0 => Constraint::Length(u16::try_from(icon_width).unwrap_or(u16::MAX)),
        1 => Constraint::Fill(1),
        _ => Constraint::Length(
            u16::try_from(
                LANG_HEADERS[index - 2]
                    .width()
                    .max(totals[index - 2].to_string().width()),
            )
            .unwrap_or(u16::MAX),
        ),
    })
}

fn lang_header_row() -> Row<'static> {
    let style = Style::default()
        .fg(theme_roles::column_header_color())
        .add_modifier(Modifier::BOLD);
    let mut cells = vec![Cell::from(""), Cell::from("")];
    cells.extend(LANG_HEADERS.map(|header| {
        Cell::from(ratatui::text::Line::from(header).alignment(Alignment::Right)).style(style)
    }));
    Row::new(cells)
}

fn lang_footer_row(stats: &LanguageStats) -> Row<'static> {
    let number_bold_style = Style::default()
        .fg(title_color())
        .add_modifier(Modifier::BOLD);
    let data_bold_style = Style::default()
        .fg(text_default())
        .add_modifier(Modifier::BOLD);
    let [
        total_files,
        total_code,
        total_comments,
        total_blanks,
        grand_total,
    ] = lang_totals(stats);
    Row::new(vec![
        Cell::from(""),
        Cell::from("Total").style(data_bold_style),
        Cell::from(ratatui::text::Line::from(total_files.to_string()).alignment(Alignment::Right))
            .style(number_bold_style),
        Cell::from(ratatui::text::Line::from(total_code.to_string()).alignment(Alignment::Right))
            .style(data_bold_style),
        Cell::from(
            ratatui::text::Line::from(total_comments.to_string()).alignment(Alignment::Right),
        )
        .style(data_bold_style),
        Cell::from(ratatui::text::Line::from(total_blanks.to_string()).alignment(Alignment::Right))
            .style(data_bold_style),
        Cell::from(ratatui::text::Line::from(grand_total.to_string()).alignment(Alignment::Right))
            .style(number_bold_style),
    ])
}

fn lang_entry_row(render_entry: &LangRenderEntry<'_>, name_width: usize) -> Row<'static> {
    let (lang_entry, parent) = match render_entry {
        LangRenderEntry::Language(lang_entry) => (*lang_entry, None),
        LangRenderEntry::Subtotal {
            entry: lang_entry,
            language,
        } => (*lang_entry, Some(*language)),
    };
    let icon = if parent.is_none() {
        format!(" {}", language_icon(&lang_entry.language))
    } else {
        String::new()
    };
    let label = if parent.is_some() {
        format!("{LANG_SUBTOTAL_MARK}{}", lang_entry.language)
    } else {
        lang_entry.language.clone()
    };
    let name = render::truncate_with_ellipsis(&label, name_width, "\u{2026}");
    let total = lang_entry.code + lang_entry.comments + lang_entry.blanks;
    let is_subtotal = parent.is_some();
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
        Cell::from(
            ratatui::text::Line::from(lang_entry.files.to_string()).alignment(Alignment::Right),
        )
        .style(num_style),
        Cell::from(
            ratatui::text::Line::from(lang_entry.code.to_string()).alignment(Alignment::Right),
        )
        .style(data_style),
        Cell::from(
            ratatui::text::Line::from(lang_entry.comments.to_string()).alignment(Alignment::Right),
        )
        .style(data_style),
        Cell::from(
            ratatui::text::Line::from(lang_entry.blanks.to_string()).alignment(Alignment::Right),
        )
        .style(data_style),
        Cell::from(ratatui::text::Line::from(total.to_string()).alignment(Alignment::Right))
            .style(num_style),
    ])
}

enum LangRenderEntry<'a> {
    Language(&'a LangEntry),
    Subtotal {
        entry:    &'a LangEntry,
        language: &'a str,
    },
}

fn flatten_lang_entries(stats: &LanguageStats) -> Vec<LangRenderEntry<'_>> {
    let mut rows = Vec::new();
    for entry in &stats.entries {
        rows.push(LangRenderEntry::Language(entry));
        rows.extend(
            entry
                .children
                .iter()
                .map(|child| LangRenderEntry::Subtotal {
                    entry:    child,
                    language: &entry.language,
                }),
        );
    }
    rows
}

fn build_lang_rows(
    viewport: &Viewport,
    entries: &[LangRenderEntry<'_>],
    name_width: usize,
    focus: PaneFocusState,
    mode: ColorMode,
) -> Vec<Row<'static>> {
    entries
        .iter()
        .enumerate()
        .map(|(row_index, render_entry)| {
            lang_entry_row(render_entry, name_width)
                .style(tui_pane::selection_state(viewport, row_index, focus).overlay_style(mode))
        })
        .collect()
}

fn render_lang_table(
    frame: &mut Frame,
    pane: &mut LangPane,
    rows: Vec<Row<'static>>,
    widths: [Constraint; 7],
    body_area: Rect,
    mode: ColorMode,
) {
    let cursor = pane.viewport.pos();
    let table = Table::new(rows, widths)
        .column_spacing(LANG_COLUMN_GAP)
        .row_highlight_style(Style::default());
    let mut table_state = TableState::default().with_selected(Some(cursor));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(table, body_area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
    for slot in 0..usize::from(body_area.height)
        .min(pane.viewport.len().saturating_sub(table_state.offset()))
    {
        let area = Rect::new(
            body_area.x,
            body_area.y + u16::try_from(slot).unwrap_or(u16::MAX),
            body_area.width,
            1,
        );
        theme_roles::paint_selection(
            frame,
            area,
            tui_pane::selection_state(
                &pane.viewport,
                table_state.offset() + slot,
                pane.focus.pane_focus_state,
            ),
            mode,
        );
    }
}

fn lang_pane_title(stats: Option<&LanguageStats>, pane: &LangPane) -> String {
    let len = stats.map_or(0, |stats| stats.entries.len());
    let cursor = matches!(pane.focus.pane_focus_state, PaneFocusState::Active)
        .then(|| stats.and_then(|stats| parent_position(stats, pane.viewport.pos())))
        .flatten();
    tui_pane::pane_title("Languages", &PaneTitleCount::Single { len, cursor })
}

/// Draw the Languages pane from its data and presentation state.
pub fn render_lang_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut LangPane,
    pane_render_ctx: &PaneRenderCtx<'_>,
) -> PaneFrameChrome {
    let mode = pane_render_ctx.color_mode;
    let pane_focus_state = pane.focus.pane_focus_state;
    let is_focused = pane.focus.is_focused();
    let PaneRenderCtx {
        project_list: projects,
        selected_project_path,
        ..
    } = pane_render_ctx;

    if pane_render_ctx.project_presentation() == ProjectPresentation::Deleted {
        pane.viewport.clear_surface();
        return PaneFrameChrome {
            title: " Languages — deleted ".into(),
            focused: is_focused,
            ..PaneFrameChrome::default()
        };
    }
    let lang_stats = selected_language_stats(projects, *selected_project_path);

    let title = lang_pane_title(lang_stats, pane);
    let mut inner = tui_pane::frame_inner(area);
    inner.width = inner.width.saturating_sub(LANG_RIGHT_MARGIN);
    let mut pane_frame_chrome = PaneFrameChrome {
        title,
        focused: is_focused,
        ..PaneFrameChrome::default()
    };

    let Some(stats) = lang_stats else {
        pane.viewport.clear_surface();
        frame.render_widget(Paragraph::new("  Scanning..."), inner);
        return pane_frame_chrome;
    };

    if stats.entries.is_empty() {
        pane.viewport.clear_surface();
        frame.render_widget(Paragraph::new("  No source files detected"), inner);
        return pane_frame_chrome;
    }

    if inner.height < 2 {
        pane.viewport.clear_surface();
        return pane_frame_chrome;
    }

    let widths = lang_table_widths(stats);
    let entries = flatten_lang_entries(stats);
    let entry_count = entries.len();

    let name_width = usize::from(inner.width).saturating_sub(fixed_columns_width(&widths));

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Table::new([lang_header_row()], widths).column_spacing(LANG_COLUMN_GAP),
        header_area,
    );

    let content_below_header = inner.height.saturating_sub(1);
    let rows_needed = u16::try_from(entry_count + 1).unwrap_or(u16::MAX);
    let pin_footer = rows_needed > content_below_header;

    let mut rows = build_lang_rows(&pane.viewport, &entries, name_width, pane_focus_state, mode);

    if pin_footer {
        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        frame.render_widget(
            Table::new([lang_footer_row(stats)], widths).column_spacing(LANG_COLUMN_GAP),
            footer_area,
        );
        let body_height = inner.height.saturating_sub(2);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        let viewport = &mut pane.viewport;
        viewport.set_len(rows.len());
        viewport.set_content_area(body_area);
        viewport.set_viewport_rows(usize::from(body_area.height));
        render_lang_table(frame, pane, rows, widths, body_area, mode);
    } else {
        rows.push(lang_footer_row(stats));
        let body_height = inner.height.saturating_sub(1);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        let viewport = &mut pane.viewport;
        viewport.set_len(entry_count);
        viewport.set_content_area(body_area);
        viewport.set_viewport_rows(usize::from(body_area.height));
        render_lang_table(frame, pane, rows, widths, body_area, mode);
    }

    pane_frame_chrome
        .labels
        .extend(tui_pane::overflow_affordance_label(
            area,
            pane.viewport.overflow(),
            Style::default().fg(label_color()),
        ));

    pane_frame_chrome
}

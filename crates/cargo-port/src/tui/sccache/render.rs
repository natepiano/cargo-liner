use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::Action;
use tui_pane::NavAction;
use tui_pane::PaneFocusState;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ViewportOverflow;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::selection_state;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::constants::CONTENT_WIDTH_PADDING;
use super::constants::POPUP_BORDER_HEIGHT;
use super::constants::POPUP_HORIZONTAL_MARGIN;
use super::constants::POPUP_MIN_WIDTH;
use super::constants::POPUP_VERTICAL_MARGIN;
use super::pane::SccachePane;
use super::pane::SccacheSelectableLine;
use super::pane::SccacheStatus;
use super::pane::SccacheTarget;
use super::stats;
use super::stats::ParsedStatLine;
use super::stats::ValueAlignment;
use crate::tui::app::App;
use crate::tui::integration::AppGlobalAction;
use crate::tui::integration::AppPaneId;
use crate::tui::keymap::SccacheAction;
use crate::tui::overlays::PopupFrame;

struct SccacheLines {
    lines:            Vec<Line<'static>>,
    selectable_lines: Vec<SccacheSelectableLine>,
}

pub(in crate::tui) fn render_sccache_popup(frame: &mut Frame<'_>, app: &mut App) {
    let SccacheLines {
        lines,
        selectable_lines,
    } = build_lines(&app.overlays.sccache_pane);
    let hints = shortcut_hints(app);
    let width = content_width(&lines)
        .max(content_width(std::slice::from_ref(&hints)))
        .min(frame.area().width.saturating_sub(POPUP_HORIZONTAL_MARGIN));
    let height = popup_height(lines.len() + 1, frame.area().height);
    let popup = PopupFrame {
        title: Some(" Sccache Stats ".to_string()),
        border_color: title_color(),
        width,
        height,
    }
    .render_with_areas(frame);
    let inner = Rect {
        height: popup.inner.height.saturating_sub(1),
        ..popup.inner
    };
    if popup.inner.height > 0 {
        frame.render_widget(
            Paragraph::new(hints),
            Rect::new(popup.inner.x, inner.bottom(), popup.inner.width, 1),
        );
    }

    let line_count = lines.len();
    app.overlays
        .sccache_pane
        .set_selectable_lines(selectable_lines);
    let visible_height = usize::from(inner.height);
    // A status-only popup starts at its first line.
    let scroll_offset = app.overlays.sccache_pane.selected_line().map_or(0, |line| {
        keep_visible_scroll_offset(line.line_index, visible_height, line_count)
    });
    let viewport = app.overlays.sccache_pane.viewport_mut();
    viewport.set_content_area(inner);
    viewport.set_viewport_rows(visible_height);
    viewport.set_scroll_offset(scroll_offset);

    let paragraph = Paragraph::new(lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(paragraph, inner);
    render_overflow_affordance(
        frame,
        popup.outer,
        ViewportOverflow::new(line_count, scroll_offset, visible_height, scroll_offset),
        Style::default().fg(label_color()),
    );
}

fn shortcut_hints(app: &App) -> Line<'static> {
    let keymap = &app.framework_keymap;
    let mut hints = Vec::new();
    if let Some(keys) =
        keymap.key_for_toml_key(AppPaneId::Sccache, SccacheAction::Cancel.toml_key())
    {
        hints.push(format!(
            "{} {}",
            keys.display_short(),
            SccacheAction::Cancel.bar_label()
        ));
    }
    if let Some(scope) = keymap.globals::<AppGlobalAction>()
        && let Some(keys) = scope.key_for(AppGlobalAction::Copy)
    {
        hints.push(format!(
            "{} {}",
            keys.display_short(),
            AppGlobalAction::Copy.bar_label()
        ));
    }
    if let Some(scope) = keymap.navigation() {
        for (backward, forward, label) in [
            (NavAction::Up, NavAction::Down, "nav"),
            (NavAction::Home, NavAction::End, "ends"),
            (NavAction::PageUp, NavAction::PageDown, "page"),
            (NavAction::HalfPageUp, NavAction::HalfPageDown, "half page"),
        ] {
            match (scope.key_for(backward), scope.key_for(forward)) {
                (Some(first), Some(second)) => hints.push(format!(
                    "{}/{} {label}",
                    first.display_short(),
                    second.display_short()
                )),
                (Some(keys), None) => {
                    hints.push(format!("{} {}", keys.display_short(), backward.bar_label()));
                },
                (None, Some(keys)) => {
                    hints.push(format!("{} {}", keys.display_short(), forward.bar_label()));
                },
                (None, None) => {},
            }
        }
    }
    Line::from(hints.join(" · "))
}

fn build_lines(pane: &SccachePane) -> SccacheLines {
    let mut lines = Vec::new();
    let mut selectable_lines = Vec::new();
    match pane.status() {
        SccacheStatus::Loading => {
            push_header(&mut lines, "Status");
            push_item(&mut lines, "Loading sccache stats");
        },
        SccacheStatus::Ready { lines: stat_rows } => {
            push_header(&mut lines, "Stats");
            push_stat_lines(&mut lines, &mut selectable_lines, pane, stat_rows);
        },
        SccacheStatus::Failed { lines: errors } => {
            push_header(&mut lines, "Error");
            push_stat_lines(&mut lines, &mut selectable_lines, pane, errors);
        },
    }
    SccacheLines {
        lines,
        selectable_lines,
    }
}

fn push_header(lines: &mut Vec<Line<'static>>, text: &'static str) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{text}:"),
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
}

fn push_item(lines: &mut Vec<Line<'static>>, text: impl Into<String>) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_ITEM_INDENT),
        Span::styled(text.into(), Style::default().fg(text_default())),
    ]));
}

fn push_subheader(lines: &mut Vec<Line<'static>>, text: String) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            text,
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
}

fn push_stat_lines(
    lines: &mut Vec<Line<'static>>,
    selectable_lines: &mut Vec<SccacheSelectableLine>,
    pane: &SccachePane,
    raw_lines: &[String],
) {
    let parsed = stats::parse_stat_lines(raw_lines);
    let alignment = ValueAlignment::for_lines(&parsed);
    for line in parsed {
        push_parsed_stat_line(lines, selectable_lines, pane, line, alignment);
    }
}

fn push_parsed_stat_line(
    lines: &mut Vec<Line<'static>>,
    selectable_lines: &mut Vec<SccacheSelectableLine>,
    pane: &SccachePane,
    parsed: ParsedStatLine,
    alignment: ValueAlignment,
) {
    match parsed {
        ParsedStatLine::Field { label, value } => {
            let copy_value = value.clone();
            let target = selectable_lines.len();
            selectable_lines.push(SccacheSelectableLine {
                line_index: lines.len(),
                target:     SccacheTarget {
                    label: label.clone(),
                    value: copy_value,
                },
            });
            let selection = selection_state(&pane.viewport, target, PaneFocusState::Active);
            let label_width = stat_label_width();
            lines.push(Line::from(vec![
                Span::styled(SECTION_ITEM_INDENT, selection.patch(Style::default())),
                Span::styled(
                    format!("{label:<label_width$} "),
                    selection.patch(Style::default().fg(label_color())),
                ),
                Span::styled(
                    alignment.format(&value),
                    selection.patch(Style::default().fg(text_default())),
                ),
            ]));
        },
        ParsedStatLine::Subheader { text, context: _ } => push_subheader(lines, text),
        ParsedStatLine::Text(text) => push_item(lines, text),
    }
}

const fn stat_label_width() -> usize { 36 }

fn content_width(lines: &[Line<'_>]) -> u16 {
    lines
        .iter()
        .map(line_width)
        .max()
        .and_then(|width| u16::try_from(width.saturating_add(CONTENT_WIDTH_PADDING)).ok())
        .unwrap_or(POPUP_MIN_WIDTH)
        .max(POPUP_MIN_WIDTH)
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    let max_height = area_height.saturating_sub(POPUP_VERTICAL_MARGIN);
    content_height
        .saturating_add(POPUP_BORDER_HEIGHT)
        .min(max_height)
        .max(POPUP_BORDER_HEIGHT.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;
    use tui_pane::Keymap;
    use tui_pane::NavAction;
    use tui_pane::VimMode;

    use super::*;
    use crate::sccache::StatsResult;
    use crate::tui::integration;
    use crate::tui::test_support;

    #[test]
    fn clicking_a_scrolled_stat_line_copies_its_own_value_across_headings() {
        let mut pane = SccachePane::new();
        let request = pane.start_loading();
        pane.apply_result(
            request,
            StatsResult::Ready(vec![
                "Cache hits  7".to_string(),
                String::new(),
                "Compiler:".to_string(),
                "Cache misses  3".to_string(),
                "Some explanatory text".to_string(),
            ]),
        );
        let SccacheLines {
            lines,
            selectable_lines,
        } = build_lines(&pane);
        assert!(lines[1].to_string().contains("Cache hits"));
        assert!(lines[4].to_string().contains("Cache misses"));
        pane.set_selectable_lines(selectable_lines);
        pane.viewport.set_content_area(Rect::new(10, 20, 60, 4));
        assert_eq!(pane.row_at(Position::new(10, 20)), None);
        assert_eq!(pane.row_at(Position::new(10, 21)), Some(0));
        pane.viewport.set_scroll_offset(2);
        assert_eq!(pane.row_at(Position::new(10, 20)), None);
        assert_eq!(pane.row_at(Position::new(10, 21)), None);
        let clicked_row = pane.row_at(Position::new(10, 22));
        assert_eq!(clicked_row, Some(1));
        if let Some(row) = clicked_row {
            pane.viewport.set_pos(row);
        }
        assert_eq!(pane.selected_line().map(|line| line.line_index), Some(4));
        assert_eq!(
            pane.selected_copy_value(),
            Some(&SccacheTarget {
                label: "Cache misses".to_string(),
                value: "3".to_string(),
            })
        );
        pane.navigate(NavAction::Up);
        assert_eq!(
            pane.selected_copy_value(),
            Some(&SccacheTarget {
                label: "Cache hits".to_string(),
                value: "7".to_string(),
            })
        );
        assert_eq!(pane.row_at(Position::new(10, 23)), None);
        assert_eq!(pane.row_at(Position::new(9, 22)), None);
    }

    #[test]
    fn status_only_lines_have_no_selection_or_copy_value() {
        let mut pane = SccachePane::new();
        let SccacheLines {
            lines,
            selectable_lines,
        } = build_lines(&pane);
        assert_eq!(lines.len(), 2);
        pane.set_selectable_lines(selectable_lines);
        pane.viewport.set_content_area(Rect::new(0, 0, 60, 4));
        assert_eq!(pane.viewport.len(), 0);
        assert_eq!(pane.selected_line(), None);
        assert_eq!(pane.selected_copy_value(), None);
        assert_eq!(pane.row_at(Position::new(0, 1)), None);
    }
    #[test]
    #[expect(
        clippy::expect_used,
        reason = "tests should panic on unexpected values"
    )]
    fn sccache_render_evidence_has_registered_hints_and_no_copy_targets_in_the_footer() {
        let mut app = test_support::make_app(&[]);
        app.framework_keymap = std::rc::Rc::new(
            integration::build_framework_keymap(
                Keymap::builder().vim_mode(VimMode::Disabled),
                &mut app.framework,
            )
            .expect("default keymap"),
        );
        let before = build_lines(&app.overlays.sccache_pane)
            .lines
            .iter()
            .map(|line| line.to_string().trim().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(before, "Status:\nLoading sccache stats");
        let hints = shortcut_hints(&app).to_string();
        assert_eq!(
            hints,
            "Esc close · y copy · ↑/↓ nav · home/end ends · pageup/pagedown page"
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("terminal");
        terminal
            .draw(|frame| render_sccache_popup(frame, &mut app))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains(&hints));
        assert!(rendered.contains("Loading sccache stats"));
        assert_eq!(app.overlays.sccache_pane.viewport.len(), 0);
        println!("Before Sccache body (unchanged build_lines):\n{before}");
        println!("After Sccache popup (100x12):\n{rendered}");

        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("keymap.toml");
        std::fs::write(&path, "[sccache]\ncancel = [\"ctrl-w\", \"z z\"]\n[global]\ncopy = \"\"\n[navigation]\nup = \"ctrl-k\"\ndown = \"\"").expect("rebound hints");
        let builder = Keymap::builder()
            .vim_mode(VimMode::Disabled)
            .load_toml(path.clone())
            .expect("load rebound keys");
        app.framework_keymap = std::rc::Rc::new(
            integration::build_framework_keymap(builder, &mut app.framework)
                .expect("build rebound keymap"),
        );
        assert_eq!(
            shortcut_hints(&app).to_string(),
            "⌃w close · ⌃k/↓ nav · home/end ends · pageup/pagedown page"
        );
        std::fs::write(&path, "[sccache]\ncancel = \"\"").expect("unbind close");
        let builder = Keymap::builder()
            .vim_mode(VimMode::Disabled)
            .load_toml(path)
            .expect("load unbound close");
        app.framework_keymap = std::rc::Rc::new(
            integration::build_framework_keymap(builder, &mut app.framework)
                .expect("build unbound keymap"),
        );
        assert!(!shortcut_hints(&app).to_string().contains("close"));
    }
}

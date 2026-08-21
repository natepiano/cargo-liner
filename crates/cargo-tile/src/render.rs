//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;
use tui_pane::BarPalette;
use tui_pane::FrameworkOverlayId;
use tui_pane::Keymap;
use tui_pane::KeymapPane;
use tui_pane::PaneFocusState;
use tui_pane::PopupFrame;
use tui_pane::RenderFocus;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ScanIndicator;
use tui_pane::SettingsRenderOptions;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::accent_color;
use tui_pane::active_border_color;
use tui_pane::error_color;
use tui_pane::hover_focus_color;
use tui_pane::inline_error_color;
use tui_pane::label_color;
use tui_pane::render_status_line;
use tui_pane::secondary_text_color;
use tui_pane::selection_style;
use tui_pane::status_bar_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;

use crate::app::App;
use crate::constants::PLACEHOLDER_BODY;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::SETTINGS_POPUP_WIDTH;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::globals::AppGlobalAction;
use crate::settings;

/// Draw one frame: panes fill the terminal above the status line, and an
/// open overlay floats above both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    draw_panes(frame, app, body);
    draw_status_line(frame, app, keymap, status);

    match app.framework.overlay() {
        Some(FrameworkOverlayId::Settings) => draw_settings(frame, app),
        Some(FrameworkOverlayId::Keymap) => draw_keymap(frame, app, keymap),
        Some(FrameworkOverlayId::GlobalShortcuts) => draw_global_shortcuts(frame, app, keymap),
        _ => (),
    }
}

/// Draw the app's panes. One placeholder pane in the template; a real
/// app splits `area` with [`Layout`] and draws one pane per
/// [`crate::app::AppPaneId`].
fn draw_panes(frame: &mut Frame, _app: &App, area: Rect) {
    let block = Block::bordered()
        .border_style(Style::default().fg(active_border_color()))
        .title(Span::styled(
            " cargo-tile ",
            Style::default().fg(title_color()),
        ));
    let body = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{SECTION_HEADER_INDENT}{PLACEHOLDER_BODY}"),
            Style::default().fg(text_default()),
        )),
    ])
    .block(block);
    frame.render_widget(body, area);
}

/// Draw the framework status line.
///
/// The shortcut strip on the right is composed by the framework from the
/// slots named here, so `?` lands in the bottom-right corner and stays
/// bound to whatever `keymap.toml` maps it to.
fn draw_status_line(frame: &mut Frame, app: &App, keymap: &Keymap<App>, area: Rect) {
    let globals = [StatusLineGlobal::global_shortcuts_help()];
    let status = StatusLine::new(
        app.started.elapsed().as_secs(),
        ScanIndicator::Hidden,
        &[],
        &globals,
    );
    render_status_line::<App, AppGlobalAction>(
        frame,
        area,
        app,
        keymap,
        &app.framework,
        &bar_palette(),
        &status,
    );
}

/// Status-line styling, drawn from the active theme.
fn bar_palette() -> BarPalette {
    let enabled_key_style = Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD);
    let disabled_key_style = Style::default()
        .fg(secondary_text_color())
        .add_modifier(Modifier::BOLD);
    BarPalette {
        status_line_style: Style::default().bg(status_bar_color()).fg(text_default()),
        status_activity_style: enabled_key_style,
        status_label_style: Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        status_value_style: Style::default().fg(text_default()),
        enabled_key_style,
        enabled_label_style: Style::default().fg(text_default()),
        disabled_key_style,
        disabled_label_style: Style::default().fg(secondary_text_color()),
        separator_style: Style::default(),
    }
}

/// Draw the framework's keymap overlay: every registered action, its
/// scope, and the key it currently resolves to.
fn draw_keymap(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    app.framework.keymap_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    let inputs = KeymapPane::prepare_overlay_inputs(app, keymap);
    app.framework
        .keymap_pane
        .render_overlay(frame, frame.area(), &inputs);
}

/// Draw the framework's global-shortcuts overlay — the `?` popup.
fn draw_global_shortcuts(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    app.framework.global_shortcuts_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    app.framework
        .global_shortcuts_pane
        .render(frame, frame.area(), keymap);
}

/// Popup width that fits the widest row plus borders, never narrower
/// than [`SETTINGS_POPUP_WIDTH`]. The caller clamps it to the terminal.
fn fitted_width(widest_row: usize) -> u16 {
    let width = u16::try_from(widest_row.saturating_add(usize::from(POPUP_CHROME_WIDTH)))
        .unwrap_or(u16::MAX);
    width.max(SETTINGS_POPUP_WIDTH)
}

/// Draw the framework settings overlay, sized to its content and
/// clamped to the terminal so a resize never clips a row.
fn draw_settings(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let built = settings::rows(app);
    // The popup centers itself and clamps to the frame, so the rows have
    // to be laid out for the width that survives that clamp.
    let width = fitted_width(built.widest_row).min(area.width);
    let content_width = usize::from(width.saturating_sub(POPUP_CHROME_WIDTH));
    let options = SettingsRenderOptions {
        focus: PaneFocusState::Active,
        inline_error: None,
        content_width,
        section_header_indent: SECTION_HEADER_INDENT,
        section_item_indent: SECTION_ITEM_INDENT,
        title_style: Style::default().fg(title_color()),
        label_style: Style::default().fg(label_color()),
        muted_style: Style::default().fg(label_color()),
        success_style: Style::default().fg(success_color()),
        error_style: Style::default().fg(error_color()),
        inline_error_style: Style::default().fg(inline_error_color()),
        active_style: selection_style(PaneFocusState::Active),
        remembered_style: selection_style(PaneFocusState::Remembered),
        hovered_style: Style::default().bg(hover_focus_color()),
    };
    let rendered = app
        .framework
        .settings_pane
        .render_rows(&built.rows, options);
    let line_count = u16::try_from(rendered.lines.len()).unwrap_or(u16::MAX);
    let height = line_count
        .saturating_add(POPUP_CHROME_HEIGHT)
        .min(area.height);
    let popup = PopupFrame {
        title: Some(" Settings ".to_string()),
        border_color: active_border_color(),
        width,
        height,
    }
    .render_with_areas(frame);

    let viewport = app.framework.settings_pane.viewport_mut();
    viewport.set_len(rendered.selectable_count);
    viewport.set_content_area(popup.inner);
    viewport.set_viewport_rows(usize::from(popup.inner.height));
    frame.render_widget(Paragraph::new(rendered.lines), popup.inner);
}

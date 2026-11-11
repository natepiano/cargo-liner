use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::symbols::line::VERTICAL;
use ratatui::symbols::line::VERTICAL_LEFT;
use ratatui::symbols::line::VERTICAL_RIGHT;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::fallback_toast_palette;
use super::format;
use crate::ACTIVITY_SPINNER;
use crate::ToastPlacement;
use crate::ToastSettings;
use crate::ViewportOverflow;
use crate::constants::TOAST_BODY_HORIZONTAL_PADDING;
use crate::constants::TOAST_ROW_GAP;
use crate::constants::TOAST_SIDE_GAP;
use crate::inactive_border_color;
use crate::inactive_title_color;
use crate::render_overflow_affordance;
use crate::title_color;
use crate::toasts::ToastCountdownState;
use crate::toasts::ToastFocus;
use crate::toasts::ToastLingerState;
use crate::toasts::ToastStyle;
use crate::toasts::ToastView;
use crate::toasts::TrackedItemActivity;
use crate::toasts::TrackedItemElapsed;
use crate::toasts::TrackedItemLifecycle;
use crate::toasts::TrackedItemView;
use crate::toasts::manager;

pub(super) fn render_toast(
    frame: &mut Frame,
    area: Rect,
    card: Rect,
    toast: &ToastView,
    focus: ToastFocus,
    page_height: u16,
) -> Rect {
    let card = card.intersection(area);
    clear_toast_surroundings(frame, area, card);

    let focused = focus == ToastFocus::PaneFocusedOn(toast.id());
    let toast_style = toast.style();
    let body_style = toast_body_style(toast_style);
    // Transparent cards keep the fallback colours. Opaque cards use the theme colours over the
    // theme background. Normal cards use the pane border and title colours.
    let border_style = match toast_style {
        ToastStyle::Error | ToastStyle::Success | ToastStyle::Warning => body_style,
        ToastStyle::Normal => {
            if focused {
                Style::default().fg(title_color())
            } else {
                Style::default().fg(inactive_border_color())
            }
        },
    };
    let text_style = match toast_style {
        ToastStyle::Error | ToastStyle::Success | ToastStyle::Warning => body_style,
        ToastStyle::Normal => {
            if focused {
                Style::default()
                    .fg(title_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(inactive_title_color())
            }
        },
    };
    let close_text = "[x]";
    let close_width = u16::try_from(close_text.len()).unwrap_or(u16::MAX);
    let raw_title = toast_title(toast);
    let title_max = usize::from(card.width.saturating_sub(close_width + 5));
    // Use the last border cell before the corner when it keeps the title whole.
    let title_uses_border_cell = raw_title.width() == title_max.saturating_add(1);
    let close_padding = u16::from(!title_uses_border_cell);
    let title_max = title_max.saturating_add(usize::from(title_uses_border_cell));
    let title = fitted_toast_title(toast, &raw_title, title_max);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    if !title.is_empty() {
        block = block.title(Span::styled(format!(" {title} "), text_style));
    }
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let close_rect = Rect {
        x:      card.x + card.width.saturating_sub(close_width + 1 + close_padding),
        y:      card.y,
        width:  close_width + close_padding,
        height: 1,
    };
    render_close_button(frame, close_rect, text_style);

    let mut page_area = card;
    if let ToastCountdownState::CountingDown {
        remaining_secs: secs,
    } = toast.countdown()
    {
        let countdown = format!(" Closing in {secs} ");
        let countdown_width = u16::try_from(countdown.len()).unwrap_or(u16::MAX);
        let countdown_rect = Rect {
            x:      card.x + card.width.saturating_sub(countdown_width + 1),
            y:      card.y + card.height.saturating_sub(1),
            width:  countdown_width,
            height: 1,
        };
        page_area.width = countdown_rect.x.saturating_sub(card.x).saturating_add(1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(countdown, toast_label_style()))),
            countdown_rect,
        );
    }

    if inner.height > 0 {
        let alloc_interior = card.height.saturating_sub(2);
        let overflow = render_toast_body(
            frame,
            toast,
            body_style,
            toast_body_area(inner),
            alloc_interior,
            page_height.saturating_sub(2),
        );
        render_page_footer(frame, page_area, overflow, toast_style, border_style);
    }

    close_rect
}

/// Select text colours for the card background.
fn toast_body_style(toast_style: ToastStyle) -> Style {
    if crate::transparent_background() {
        let palette = fallback_toast_palette();
        match toast_style {
            ToastStyle::Error => Style::default().fg(palette.error),
            ToastStyle::Success => Style::default().fg(palette.success),
            ToastStyle::Warning => Style::default().fg(palette.warning),
            ToastStyle::Normal => Style::default(),
        }
    } else {
        let color = match toast_style {
            ToastStyle::Error => crate::error_color(),
            ToastStyle::Success => crate::success_color(),
            ToastStyle::Warning => crate::warning_color(),
            ToastStyle::Normal => crate::text_default(),
        };
        Style::default().fg(color)
    }
}

/// Keep secondary card text readable over opaque backgrounds.
fn toast_label_style() -> Style {
    let color = if crate::transparent_background() {
        fallback_toast_palette().label
    } else {
        crate::text_default()
    };
    Style::default().fg(color)
}

/// Use the theme text colour over the card background.
fn render_page_footer(
    frame: &mut Frame,
    area: Rect,
    overflow: ViewportOverflow,
    toast_style: ToastStyle,
    border_style: Style,
) {
    let style = if toast_style == ToastStyle::Normal || !crate::transparent_background() {
        Style::default().fg(crate::text_default())
    } else {
        border_style
    };
    render_overflow_affordance(frame, area, overflow, style);
}

/// Omit a title when its first word cannot fit.
fn fitted_toast_title(toast: &ToastView, raw_title: &str, width: usize) -> String {
    let word = toast.title().split_whitespace().next().unwrap_or("");
    let prefix = raw_title.width().saturating_sub(toast.title().width());
    let cut_mark = usize::from(raw_title.width() > width);
    if word.is_empty() || word.width() + prefix + cut_mark > width {
        return String::new();
    }
    let title = format::truncate_with_ellipsis(raw_title, width);
    if title
        .split_whitespace()
        .any(|drawn| drawn == word || drawn.strip_suffix('…') == Some(word))
    {
        title
    } else {
        String::new()
    }
}

fn toast_title(toast: &ToastView) -> String {
    match toast.style() {
        ToastStyle::Error => format!("x {}", toast.title()),
        ToastStyle::Warning => format!("! {}", toast.title()),
        ToastStyle::Success => format!("\u{2713} {}", toast.title()),
        ToastStyle::Normal => toast.title().to_owned(),
    }
}

pub(super) fn title_card_width(toast: &ToastView) -> u16 {
    u16::try_from(toast_title(toast).width().saturating_add(8)).unwrap_or(u16::MAX)
}

pub(super) fn minimum_title_card_width(toast: &ToastView) -> u16 {
    // Keep the title whole when the close button needs the last padding cell.
    u16::try_from(toast_title(toast).width().saturating_add(7)).unwrap_or(u16::MAX)
}

/// Draw the close button with one blank cell before it.
fn render_close_button(frame: &mut Frame, close_rect: Rect, style: Style) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(" [x]", style))),
        Rect {
            x: close_rect.x.saturating_sub(1),
            width: close_rect.width.saturating_add(1),
            ..close_rect
        },
    );
}

/// Keep the stack in its corner region between the drawn pane borders.
pub(super) fn area_between_pane_borders(
    frame: &mut Frame,
    area: Rect,
    settings: &ToastSettings,
    required_height: u16,
) -> Rect {
    let area = area.intersection(frame.area());
    if area.is_empty() {
        return area;
    }
    let width = manager::toast_card_width(settings, area.width);
    let left = area
        .right()
        .saturating_sub(width)
        .saturating_sub(TOAST_SIDE_GAP.saturating_mul(2))
        .max(area.left());
    let mut selected = Rect::new(area.x, area.y, area.width, 0);
    let mut start = area.top();
    for row in area.top()..=area.bottom() {
        let border = row == area.bottom()
            || (left..area.right())
                .any(|x| is_horizontal_frame(frame.buffer_mut()[(x, row)].symbol()));
        if !border {
            continue;
        }
        let candidate = Rect::new(area.x, start, area.width, row.saturating_sub(start));
        if !candidate.is_empty() {
            selected = candidate;
            if settings.placement == ToastPlacement::TopRight {
                break;
            }
        }
        start = row.saturating_add(1);
    }
    let occupied_height = required_height.min(selected.height);
    let mut occupied = match settings.placement {
        ToastPlacement::TopRight => selected.top()..selected.top().saturating_add(occupied_height),
        ToastPlacement::BottomRight => {
            selected.bottom().saturating_sub(occupied_height)..selected.bottom()
        },
    };
    if occupied.any(|y| is_vertical_frame(frame.buffer_mut()[(selected.right() - 1, y)].symbol())) {
        selected.width = selected.width.saturating_sub(TOAST_SIDE_GAP);
    }
    selected
}

/// Clear the card and plain gap cells. Keep pane borders and whole title rows in the gaps.
fn clear_toast_surroundings(frame: &mut Frame, area: Rect, card: Rect) {
    let clear_left = card.x.saturating_sub(TOAST_SIDE_GAP).max(area.x);
    let clear_right = card
        .right()
        .saturating_add(TOAST_SIDE_GAP)
        .min(area.right());
    let clear_top = card.top().saturating_sub(TOAST_ROW_GAP).max(area.top());
    let clear_bottom = card
        .bottom()
        .saturating_add(TOAST_ROW_GAP)
        .min(area.bottom());
    let gaps = Rect::new(
        clear_left,
        clear_top,
        clear_right.saturating_sub(clear_left),
        clear_bottom.saturating_sub(clear_top),
    );
    for y in gaps.top()..gaps.bottom() {
        let frame_row = (gaps.left()..gaps.right())
            .any(|x| is_horizontal_frame(frame.buffer_mut()[(x, y)].symbol()));
        if frame_row {
            continue;
        }
        for x in gaps.left()..gaps.right() {
            if !card.contains((x, y).into())
                && !is_vertical_frame(frame.buffer_mut()[(x, y)].symbol())
            {
                frame.buffer_mut()[(x, y)].reset();
            }
        }
    }
    frame.render_widget(Clear, card);
    if !crate::transparent_background() {
        frame
            .buffer_mut()
            .set_style(card, Style::default().bg(crate::pane_background(false)));
    }
    clear_covered_border_fragments(frame, gaps, card);
    join_adjacent_frame_columns(frame, area, card);
}

fn is_horizontal_frame(symbol: &str) -> bool {
    matches!(
        symbol,
        "─" | "━"
            | "═"
            | "┌"
            | "┐"
            | "└"
            | "┘"
            | "╭"
            | "╮"
            | "╰"
            | "╯"
            | "┏"
            | "┓"
            | "┗"
            | "┛"
            | "╔"
            | "╗"
            | "╚"
            | "╝"
            | "├"
            | "┤"
            | "┬"
            | "┴"
            | "┼"
            | "┣"
            | "┫"
            | "┳"
            | "┻"
            | "╋"
            | "╠"
            | "╣"
            | "╦"
            | "╩"
            | "╬"
    )
}

fn is_vertical_frame(symbol: &str) -> bool { matches!(symbol, "│" | "┃" | "║") }

/// Remove a lone border cell only where the card covers its neighbour.
fn clear_covered_border_fragments(frame: &mut Frame, gaps: Rect, card: Rect) {
    let bounds = frame.area();
    for y in gaps.top()..gaps.bottom() {
        for x in gaps.left()..gaps.right() {
            if !is_vertical_frame(frame.buffer_mut()[(x, y)].symbol()) {
                continue;
            }
            let neighbors = y.saturating_sub(1)..=y.saturating_add(1);
            let covered = neighbors.clone().any(|row| card.contains((x, row).into()));
            let connected = neighbors.filter(|row| *row != y).any(|row| {
                bounds.contains((x, row).into()) && {
                    let symbol = frame.buffer_mut()[(x, row)].symbol();
                    is_vertical_frame(symbol) || is_horizontal_frame(symbol)
                }
            });
            if covered && !connected {
                frame.buffer_mut()[(x, y)].reset();
            }
        }
    }
}

/// Remove divider stubs that point into a toast's cleared side gaps.
fn join_adjacent_frame_columns(frame: &mut Frame, area: Rect, card: Rect) {
    let frame_area = frame.area();
    for y in card.top()..card.bottom() {
        if card.left().saturating_sub(TOAST_SIDE_GAP) == area.left()
            && area.left() > frame_area.left()
        {
            let cell = &mut frame.buffer_mut()[(area.left() - 1, y)];
            if cell.symbol() == VERTICAL_RIGHT {
                cell.set_symbol(VERTICAL);
            }
        }
        if card.right().saturating_add(TOAST_SIDE_GAP) == area.right()
            && area.right() < frame_area.right()
        {
            let cell = &mut frame.buffer_mut()[(area.right(), y)];
            if cell.symbol() == VERTICAL_LEFT {
                cell.set_symbol(VERTICAL);
            }
        }
    }
}

const fn toast_body_area(inner: Rect) -> Rect {
    Rect {
        x: inner.x.saturating_add(TOAST_BODY_HORIZONTAL_PADDING),
        width: inner
            .width
            .saturating_sub(TOAST_BODY_HORIZONTAL_PADDING.saturating_mul(2)),
        ..inner
    }
}

pub(super) struct ToastBodyRows {
    pub(super) lines: Vec<Line<'static>>,
    overflow:         ViewportOverflow,
}

fn render_toast_body(
    frame: &mut Frame,
    toast_view: &ToastView,
    body_style: Style,
    body_area: Rect,
    alloc_interior: u16,
    page_interior: u16,
) -> ViewportOverflow {
    let alloc_body = usize::from(alloc_interior);
    let has_action = toast_view.has_action() && alloc_body >= 2;
    let lines_for_body = if has_action {
        alloc_body.saturating_sub(1)
    } else {
        alloc_body
    };
    let page_rows = usize::from(page_interior)
        .saturating_sub(usize::from(toast_view.has_action() && page_interior >= 2));
    let body = if toast_view.tracked_items().is_empty() {
        body_lines_plain(
            toast_view,
            body_style,
            lines_for_body,
            body_area.width,
            page_rows,
        )
    } else {
        body_lines_tracked(
            toast_view.tracked_items(),
            body_style,
            lines_for_body,
            usize::from(body_area.width),
            page_rows,
        )
    };

    if has_action {
        let text_area = Rect {
            height: body_area.height.saturating_sub(1),
            ..body_area
        };
        frame.render_widget(Paragraph::new(body.lines).style(body_style), text_area);
        let hint_area = Rect {
            y: body_area.y + body_area.height.saturating_sub(1),
            height: 1,
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter open",
                toast_label_style().add_modifier(Modifier::ITALIC),
            ))),
            hint_area,
        );
    } else {
        frame.render_widget(Paragraph::new(body.lines).style(body_style), body_area);
    }
    body.overflow
}

fn body_lines_plain(
    toast: &ToastView,
    body_style: Style,
    lines_for_body: usize,
    width: u16,
    page_rows: usize,
) -> ToastBodyRows {
    let mut lines = toast.render_body.wrapped_lines(width, body_style);
    if let ToastLingerState::Lingering { progress } = toast.linger() {
        for line in &mut lines {
            for span in &mut line.spans {
                span.style = span.style.patch(format::fade_to_style(f64::from(progress)));
            }
        }
    }
    let overflow = ViewportOverflow::new(lines.len(), 0, page_rows, 0);
    lines.truncate(lines_for_body);
    ToastBodyRows { lines, overflow }
}

pub(super) fn body_lines_tracked(
    tracked_item_views: &[TrackedItemView],
    body_style: Style,
    lines_for_body: usize,
    line_width: usize,
    page_rows: usize,
) -> ToastBodyRows {
    let total_items = tracked_item_views.len();
    let visible_count = |rows: usize| {
        if total_items > rows {
            rows.saturating_sub(1)
        } else {
            total_items
        }
    };
    let visible = visible_count(lines_for_body);
    let visible_items = &tracked_item_views[..visible];

    let mut result = visible_items
        .iter()
        .map(|tracked_item_view| tracked_item_line(tracked_item_view, body_style, line_width))
        .collect::<Vec<_>>();
    if visible < total_items && lines_for_body > 0 {
        let overflow_style = toast_label_style().add_modifier(Modifier::ITALIC);
        result.push(Line::from(Span::styled(
            format!("(+{} more)", total_items - visible),
            overflow_style,
        )));
    }
    ToastBodyRows {
        lines:    result,
        overflow: ViewportOverflow::new(total_items, 0, visible_count(page_rows).max(1), 0),
    }
}

/// A running item's spinner takes the palette's error color while the item is
/// stalled, so a caller that reports a stall gets a red spinner without knowing
/// anything about the palette. An item that is lingering after completion shows
/// no spinner, so there is nothing to color.
fn spinner_style(
    lifecycle: TrackedItemLifecycle,
    tracked_item_activity: TrackedItemActivity,
) -> Style {
    if matches!(lifecycle, TrackedItemLifecycle::CompletedFading { .. }) {
        return Style::default();
    }
    let palette = fallback_toast_palette();
    match tracked_item_activity {
        TrackedItemActivity::Progressing => Style::default().fg(palette.accent),
        TrackedItemActivity::Stalled => Style::default().fg(palette.error),
    }
}

pub(super) fn tracked_item_line(
    tracked_item_view: &TrackedItemView,
    body_style: Style,
    line_width: usize,
) -> Line<'static> {
    const SPINNER_SLOT: usize = 4;
    let palette = fallback_toast_palette();

    let label_style = match tracked_item_view.lifecycle {
        TrackedItemLifecycle::Running | TrackedItemLifecycle::CompletedWithoutFade => body_style,
        TrackedItemLifecycle::CompletedFading { progress } => format::fade_to_style(progress),
    };
    let TrackedItemElapsed::Measured(elapsed) = tracked_item_view.elapsed else {
        return Line::from(Span::styled(
            format::truncate_with_ellipsis(&tracked_item_view.label, line_width),
            label_style,
        ));
    };

    let spinner_text = match tracked_item_view.lifecycle {
        TrackedItemLifecycle::Running | TrackedItemLifecycle::CompletedWithoutFade => {
            format!(" {} ", ACTIVITY_SPINNER.frame_at(elapsed))
        },
        TrackedItemLifecycle::CompletedFading { .. } => " ".repeat(SPINNER_SLOT),
    };
    let duration_text = format::format_elapsed(elapsed);
    let duration_suffix = format!("{duration_text} ");
    let suffix_width = duration_suffix.len();
    if tracked_item_view.label.width() + suffix_width + SPINNER_SLOT + 1 > line_width {
        let label = format::truncate_with_ellipsis(&tracked_item_view.label, line_width);
        let label_width = label.width();
        let mut spans = vec![Span::styled(label, label_style)];
        if label_width + SPINNER_SLOT < line_width {
            spans.push(Span::raw(
                " ".repeat(line_width - label_width - SPINNER_SLOT),
            ));
            spans.push(Span::styled(
                spinner_text,
                spinner_style(tracked_item_view.lifecycle, tracked_item_view.activity),
            ));
        }
        return Line::from(spans);
    }
    let label_budget = line_width.saturating_sub(suffix_width + SPINNER_SLOT + 1);
    let label = format::truncate_with_ellipsis(&tracked_item_view.label, label_budget);
    let used = label.width() + spinner_text.width() + suffix_width;
    let padding = line_width.saturating_sub(used);
    let duration_style = match tracked_item_view.lifecycle {
        TrackedItemLifecycle::Running | TrackedItemLifecycle::CompletedWithoutFade => {
            Style::default().fg(palette.title)
        },
        TrackedItemLifecycle::CompletedFading { progress } => format::fade_to_style(progress),
    };
    Line::from(vec![
        Span::styled(label, label_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            spinner_text,
            spinner_style(tracked_item_view.lifecycle, tracked_item_view.activity),
        ),
        Span::styled(duration_suffix, duration_style),
    ])
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use super::render_toast;
    use crate::ColorMode;
    use crate::ToastActivation;
    use crate::ToastId;
    use crate::toasts::ToastBody;
    use crate::toasts::ToastCountdownState;
    use crate::toasts::ToastFocus;
    use crate::toasts::ToastLingerState;
    use crate::toasts::ToastStyle;
    use crate::toasts::ToastView;
    use crate::toasts::TrackedItemActivity;
    use crate::toasts::TrackedItemElapsed;
    use crate::toasts::TrackedItemLifecycle;
    use crate::toasts::TrackedItemView;
    use crate::toasts::view::ToastExitState;

    fn toast() -> ToastView {
        let body = "Metadata\nDisk\nLanguages\nGit\nTests\nLints".to_owned();
        ToastView {
            id: ToastId(1),
            title: "Startup".to_owned(),
            render_body: ToastBody::Text(body.clone()).into(),
            body,
            style: ToastStyle::Normal,
            activation: ToastActivation::Informational,
            linger: ToastLingerState::Live,
            countdown: ToastCountdownState::NoCountdown,
            tracked_items: Vec::new(),
            min_height: 3,
            desired_height: 8,
            settled_height: 8,
            exit_state: ToastExitState::Visible,
        }
    }

    #[test]
    fn hidden_rows_keep_the_countdown_and_action_hint_whole() -> Result<(), Box<dyn Error>> {
        let mut toast = toast();
        toast.activation = ToastActivation::Opens(());
        toast.countdown = ToastCountdownState::CountingDown { remaining_secs: 12 };
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            let mut terminal = Terminal::new(TestBackend::new(40, 6))?;
            terminal.draw(|frame| {
                render_toast(
                    frame,
                    frame.area(),
                    frame.area(),
                    &toast,
                    ToastFocus::PaneUnfocused,
                    6,
                );
                mode.clear_colors(frame.buffer_mut());
            })?;
            let buffer = terminal.backend().buffer();
            let row = |y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>();
            let bottom = row(5);
            assert!(bottom.contains("1 of 2 ▼"), "{mode:?}: {bottom}");
            assert!(bottom.contains(" Closing in 12 "), "{mode:?}: {bottom}");
            assert!(row(4).contains("Enter open"));
            assert!(row(3).contains("Languages"));
        }
        Ok(())
    }

    #[test]
    fn a_narrow_tracked_card_cuts_elapsed_detail_before_the_label() -> Result<(), Box<dyn Error>> {
        let mut toast = toast();
        toast.tracked_items = vec![TrackedItemView {
            label:     "Cargo metadata".to_owned(),
            lifecycle: TrackedItemLifecycle::Running,
            elapsed:   TrackedItemElapsed::Measured(Duration::from_secs(123)),
            activity:  TrackedItemActivity::Progressing,
        }];
        for (width, detail_fits) in [(20, false), (40, true)] {
            let mut terminal = Terminal::new(TestBackend::new(width, 4))?;
            terminal.draw(|frame| {
                let card = Rect::new(0, 0, width, 4);
                render_toast(
                    frame,
                    frame.area(),
                    card,
                    &toast,
                    ToastFocus::PaneUnfocused,
                    card.height,
                );
                ColorMode::Colored.clear_colors(frame.buffer_mut());
            })?;
            let buffer = terminal.backend().buffer();
            let row: String = (1..width - 1).map(|x| buffer[(x, 1)].symbol()).collect();
            assert!(row.contains("Cargo metadata"), "{row}");
            assert!(!row.contains('…'), "{row}");
            assert_eq!(row.contains("2m 03s"), detail_fits, "{row}");
        }
        Ok(())
    }
}

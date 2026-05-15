use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use unicode_width::UnicodeWidthStr;

use super::ACCENT_COLOR;
use super::ACTIVE_BORDER_COLOR;
use super::ERROR_COLOR;
use super::LABEL_COLOR;
use super::TITLE_COLOR;
use super::WARNING_COLOR;
use super::format;
use crate::ACTIVITY_SPINNER;
use crate::toasts::ToastId;
use crate::toasts::ToastStyle;
use crate::toasts::ToastView;
use crate::toasts::TrackedItemView;

pub(super) fn render_toast(
    frame: &mut Frame,
    area: Rect,
    card: Rect,
    toast: &ToastView,
    pane_focused: bool,
    focused_toast_id: Option<ToastId>,
) -> Rect {
    let clear_rect = Rect {
        x:      card.x.saturating_sub(1),
        y:      card.y,
        width:  card
            .width
            .saturating_add(2)
            .min(area.x + area.width - card.x.saturating_sub(1)),
        height: card.height,
    };
    frame.render_widget(Clear, clear_rect);

    let focused = pane_focused && focused_toast_id == Some(toast.id());
    let is_error = toast.style() == ToastStyle::Error;
    let is_warning = toast.style() == ToastStyle::Warning;
    let accent_color = if is_error {
        ERROR_COLOR
    } else if is_warning {
        WARNING_COLOR
    } else {
        Color::White
    };
    let border_style = if focused {
        Style::default().fg(ACTIVE_BORDER_COLOR)
    } else {
        Style::default().fg(accent_color)
    };
    let text_style = if is_error || is_warning {
        Style::default().fg(accent_color)
    } else {
        border_style.add_modifier(Modifier::BOLD)
    };
    let body_style = if is_error || is_warning {
        Style::default().fg(accent_color)
    } else {
        Style::default()
    };

    let close_text = "[x]";
    let close_width = u16::try_from(close_text.len()).unwrap_or(u16::MAX);
    let title_max = usize::from(card.width.saturating_sub(close_width + 4));
    let raw_title = if is_warning {
        format!("! {}", toast.title())
    } else if is_error {
        format!("x {}", toast.title())
    } else {
        toast.title().to_owned()
    };
    let title = format::truncate(&raw_title, title_max);

    let block = Block::default()
        .title(Span::styled(format!(" {title} "), text_style))
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let close_rect = Rect {
        x:      card.x + card.width.saturating_sub(close_width + 2),
        y:      card.y,
        width:  close_width + 1,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(close_text, text_style))),
        close_rect,
    );

    if let Some(secs) = toast.remaining_secs() {
        let countdown = format!(" Closing in {secs} ");
        let countdown_width = u16::try_from(countdown.len()).unwrap_or(u16::MAX);
        let countdown_rect = Rect {
            x:      card.x + card.width.saturating_sub(countdown_width + 1),
            y:      card.y + card.height.saturating_sub(1),
            width:  countdown_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                countdown,
                Style::default().fg(LABEL_COLOR),
            ))),
            countdown_rect,
        );
    }

    if inner.height > 0 {
        let alloc_interior = card.height.saturating_sub(2);
        render_toast_body(frame, toast, body_style, inner, alloc_interior);
    }

    close_rect
}

fn render_toast_body(
    frame: &mut Frame,
    toast: &ToastView,
    body_style: Style,
    body_area: Rect,
    alloc_interior: u16,
) {
    let alloc_body = usize::from(alloc_interior);
    let has_action = toast.has_action() && alloc_body >= 2;
    let lines_for_body = if has_action {
        alloc_body.saturating_sub(1)
    } else {
        alloc_body
    };
    let lines = if toast.tracked_items().is_empty() {
        body_lines_plain(toast, body_style, lines_for_body)
    } else {
        body_lines_tracked(
            toast.tracked_items(),
            body_style,
            lines_for_body,
            usize::from(body_area.width),
        )
    };

    if has_action {
        let text_area = Rect {
            height: body_area.height.saturating_sub(1),
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(lines)
                .style(body_style)
                .wrap(Wrap { trim: false }),
            text_area,
        );
        let hint_area = Rect {
            y: body_area.y + body_area.height.saturating_sub(1),
            height: 1,
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter open",
                Style::default()
                    .fg(LABEL_COLOR)
                    .add_modifier(Modifier::ITALIC),
            ))),
            hint_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(lines)
                .style(body_style)
                .wrap(Wrap { trim: false }),
            body_area,
        );
    }
}

fn body_lines_plain(
    toast: &ToastView,
    body_style: Style,
    lines_for_body: usize,
) -> Vec<Line<'static>> {
    let body_lines = toast.body().lines().collect::<Vec<_>>();
    let total_body = body_lines.len();
    let needs_truncation = total_body > lines_for_body;
    let (visible_body, overflow_line) = if needs_truncation && lines_for_body >= 1 {
        let show = lines_for_body.saturating_sub(1);
        let remaining = total_body.saturating_sub(show);
        (
            body_lines[..show].join("\n"),
            Some(format!("(+{remaining} more)")),
        )
    } else {
        (toast.body().to_owned(), None)
    };

    let mut result = visible_body
        .lines()
        .map(|line| {
            toast.linger_progress().map_or_else(
                || Line::from(Span::styled(line.to_owned(), body_style)),
                |progress| format::fade_to_color(line, f64::from(progress)),
            )
        })
        .collect::<Vec<_>>();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(LABEL_COLOR)
            .add_modifier(Modifier::ITALIC);
        result.push(toast.linger_progress().map_or_else(
            || Line::from(Span::styled(overflow.clone(), overflow_style)),
            |progress| format::fade_to_color(&overflow, f64::from(progress)),
        ));
    }
    result
}

pub(super) fn body_lines_tracked(
    tracked: &[TrackedItemView],
    body_style: Style,
    lines_for_body: usize,
    line_width: usize,
) -> Vec<Line<'static>> {
    let total_items = tracked.len();
    let needs_truncation = total_items > lines_for_body;
    let (visible_items, overflow_line) = if needs_truncation && lines_for_body >= 1 {
        let show = lines_for_body.saturating_sub(1);
        let remaining = total_items.saturating_sub(show);
        (&tracked[..show], Some(format!("(+{remaining} more)")))
    } else {
        (tracked, None)
    };

    let mut result = visible_items
        .iter()
        .map(|item| tracked_item_line(item, body_style, line_width))
        .collect::<Vec<_>>();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(LABEL_COLOR)
            .add_modifier(Modifier::ITALIC);
        result.push(Line::from(Span::styled(overflow, overflow_style)));
    }
    result
}

pub(super) fn tracked_item_line(
    item: &TrackedItemView,
    body_style: Style,
    line_width: usize,
) -> Line<'static> {
    const SPINNER_SLOT: usize = 4;

    let label_style = item
        .linger_progress
        .map_or(body_style, format::fade_to_style);
    let Some(elapsed) = item.elapsed else {
        return Line::from(Span::styled(item.label.clone(), label_style));
    };

    let is_running = item.linger_progress.is_none();
    let spinner_text = if is_running {
        format!(" {} ", ACTIVITY_SPINNER.frame_at(elapsed))
    } else {
        " ".repeat(SPINNER_SLOT)
    };
    let duration_text = format::format_elapsed(elapsed);
    let duration_suffix = format!("{duration_text} ");
    let suffix_width = duration_suffix.len();
    let label_budget = line_width.saturating_sub(suffix_width + SPINNER_SLOT + 1);
    let label = format::truncate_with_ellipsis(&item.label, label_budget);
    let used = label.width() + spinner_text.width() + suffix_width;
    let padding = line_width.saturating_sub(used);
    let duration_style = item
        .linger_progress
        .map_or_else(|| Style::default().fg(TITLE_COLOR), format::fade_to_style);
    Line::from(vec![
        Span::styled(label, label_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            spinner_text,
            if is_running {
                Style::default().fg(ACCENT_COLOR)
            } else {
                Style::default()
            },
        ),
        Span::styled(duration_suffix, duration_style),
    ])
}

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

use super::manager::ToastStyle;
use super::manager::ToastView;
use super::manager::TrackedItemView;

/// Fade text from white to grey based on progress (0.0 = white, 1.0 = grey).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "p is clamped to [0.0, 1.0], so the result is in [TARGET, 255]"
)]
fn fade_to_style(progress: f64) -> Style {
    let p = progress.clamp(0.0, 1.0);
    let curve = p * p * p;
    // Fade from 255 (white) to 128 (grey).
    let v = 127.0f64.mul_add(-curve, 255.0) as u8;
    Style::default().fg(Color::Rgb(v, v, v))
}

fn fade_to_color<'a>(text: &str, progress: f64) -> Line<'a> {
    Line::from(Span::styled(text.to_owned(), fade_to_style(progress)))
}
use crate::tui::app::ClickAction;
use crate::tui::app::DismissTarget;
use crate::tui::constants::TOAST_GAP;
use crate::tui::constants::TOAST_WIDTH;
use crate::tui::types::ToastCardHitbox;

fn truncate(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

pub struct ToastRenderResult {
    pub dismiss_actions: Vec<ClickAction>,
    pub card_hitboxes:   Vec<ToastCardHitbox>,
}

pub fn render_toasts(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView<'_>],
    pane_focused: bool,
    focused_toast_id: Option<u64>,
) -> ToastRenderResult {
    if toasts.is_empty() {
        return ToastRenderResult {
            dismiss_actions: Vec::new(),
            card_hitboxes:   Vec::new(),
        };
    }

    let allocated = allocate_toast_heights(toasts, area.height);

    let width = TOAST_WIDTH.min(area.width);

    // Stack toasts from the bottom of the area upward.
    let mut dismiss_actions = Vec::with_capacity(toasts.len());
    let mut card_hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.y.saturating_add(area.height);
    for (toast, &alloc_height) in toasts.iter().zip(&allocated).rev() {
        if alloc_height == 0 {
            continue;
        }
        // During entrance animation, visible_lines may be less than alloc.
        let card_height = toast.visible_lines().min(alloc_height);
        if card_height == 0 {
            continue;
        }
        cursor_y = cursor_y.saturating_sub(card_height + TOAST_GAP);
        if cursor_y < area.y {
            break;
        }
        let x = area.x + area.width.saturating_sub(width);
        let card = Rect {
            x,
            y: cursor_y,
            width,
            height: card_height,
        };

        frame.render_widget(Clear, card);
        let close_rect = render_toast_card(
            frame,
            card,
            toast,
            alloc_height,
            pane_focused,
            focused_toast_id,
        );
        dismiss_actions.push(ClickAction {
            rect:   close_rect,
            target: DismissTarget::Toast(toast.id()),
        });
        card_hitboxes.push(ToastCardHitbox {
            id:        toast.id(),
            card_rect: card,
        });
    }

    dismiss_actions.reverse();
    card_hitboxes.reverse();
    ToastRenderResult {
        dismiss_actions,
        card_hitboxes,
    }
}

/// Allocate heights for each toast given total available space.
///
/// Returns a Vec parallel to `toasts` with the allocated height for each.
/// Toasts that don't fit are given 0.
fn allocate_toast_heights(toasts: &[ToastView<'_>], available: u16) -> Vec<u16> {
    let count = toasts.len();
    let mut alloc = vec![0u16; count];

    // First pass: can all minimums fit?
    let total_min: u16 = toasts
        .iter()
        .map(ToastView::min_height)
        .fold(0u16, u16::saturating_add);

    if total_min > available {
        // Not all toasts fit at minimum height. Show as many as possible.
        let mut used = 0u16;
        for (i, toast) in toasts.iter().enumerate() {
            let min_h = toast.min_height();
            if used.saturating_add(min_h) <= available {
                alloc[i] = min_h;
                used = used.saturating_add(min_h);
            }
            // Remaining toasts get 0 (not shown).
        }
        return alloc;
    }

    // All minimums fit. Start each toast at its minimum.
    for (i, toast) in toasts.iter().enumerate() {
        alloc[i] = toast.min_height();
    }
    let mut remaining = available.saturating_sub(total_min);

    // Round-robin distribute extra lines to toasts that haven't reached
    // their animated desired height (visible_lines, capped by desired).
    loop {
        if remaining == 0 {
            break;
        }
        let mut gave_any = false;
        for (i, toast) in toasts.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            let desired = toast.desired_height();
            if alloc[i] < desired {
                alloc[i] += 1;
                remaining -= 1;
                gave_any = true;
            }
        }
        if !gave_any {
            break;
        }
    }

    alloc
}

/// Render a single toast card and return the close-button rect for hit-testing.
fn render_toast_card(
    frame: &mut Frame,
    card: Rect,
    toast: &ToastView<'_>,
    alloc_height: u16,
    pane_focused: bool,
    focused_toast_id: Option<u64>,
) -> Rect {
    let focused = pane_focused && focused_toast_id == Some(toast.id());
    let is_error = toast.style() == ToastStyle::Error;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::White)
    };
    let text_style = if is_error {
        border_style
    } else {
        border_style.add_modifier(Modifier::BOLD)
    };

    // Title on the top border line, close button on the right.
    let close_text = "[x]";
    let close_width = u16::try_from(close_text.len()).unwrap_or(u16::MAX);
    let title_max = usize::from(card.width.saturating_sub(close_width + 4));
    let title = truncate(toast.title(), title_max);

    let block = Block::default()
        .title(Span::styled(format!(" {title} "), text_style))
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(card);
    frame.render_widget(block, card);

    // Close button on the top border row.
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

    if inner.height == 0 {
        return close_rect;
    }

    // Full inner area is body content (title is on border, not inner).
    let body_style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    // Countdown on the bottom border row, right-aligned.
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
                Style::default().fg(Color::DarkGray),
            ))),
            countdown_rect,
        );
    }

    // Interior lines available = alloc_height - 2 (borders).
    let alloc_interior = alloc_height.saturating_sub(2);
    render_toast_body(frame, toast, body_style, inner, alloc_interior);

    close_rect
}

/// Build body lines from plain body text.
fn body_lines_plain<'a>(
    toast: &ToastView<'_>,
    body_style: Style,
    lines_for_body: usize,
) -> Vec<Line<'a>> {
    let body_lines: Vec<&str> = toast.body().lines().collect();
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

    let mut result: Vec<Line<'_>> = visible_body
        .lines()
        .map(|l| {
            toast.linger_progress().map_or_else(
                || Line::from(Span::styled(l.to_owned(), body_style)),
                |progress| fade_to_color(l, progress),
            )
        })
        .collect();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        result.push(toast.linger_progress().map_or_else(
            || Line::from(Span::styled(overflow.clone(), overflow_style)),
            |progress| fade_to_color(&overflow, progress),
        ));
    }
    result
}

/// Build body lines from tracked items.
fn body_lines_tracked<'a>(
    tracked: &[TrackedItemView],
    body_style: Style,
    lines_for_body: usize,
    line_width: usize,
) -> Vec<Line<'a>> {
    let total_items = tracked.len();
    let needs_truncation = total_items > lines_for_body;
    let (visible_items, overflow_line) = if needs_truncation && lines_for_body >= 1 {
        let show = lines_for_body.saturating_sub(1);
        let remaining = total_items.saturating_sub(show);
        (&tracked[..show], Some(format!("(+{remaining} more)")))
    } else {
        (tracked, None)
    };

    let mut result: Vec<Line<'_>> = visible_items
        .iter()
        .map(|item| tracked_item_line(item, body_style, line_width))
        .collect();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        result.push(Line::from(Span::styled(overflow, overflow_style)));
    }
    result
}

fn tracked_item_line<'a>(item: &TrackedItemView, body_style: Style, line_width: usize) -> Line<'a> {
    let label_style = item.linger_progress.map_or(body_style, fade_to_style);
    let Some(secs) = item.elapsed_secs else {
        return Line::from(Span::styled(item.label.clone(), label_style));
    };

    let duration_suffix = format!(" {secs}s");
    let suffix_width = duration_suffix.len();
    let label_budget = line_width.saturating_sub(suffix_width);
    let label = if item.label.len() > label_budget && label_budget > 1 {
        format!("{}…", &item.label[..label_budget.saturating_sub(1)])
    } else {
        item.label.clone()
    };
    let padding = line_width
        .saturating_sub(label.len())
        .saturating_sub(suffix_width);
    let duration_style = item
        .linger_progress
        .map_or_else(|| Style::default().fg(Color::Yellow), fade_to_style);
    Line::from(vec![
        Span::styled(label, label_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(duration_suffix, duration_style),
    ])
}

fn render_toast_body(
    frame: &mut Frame,
    toast: &ToastView<'_>,
    body_style: Style,
    body_area: Rect,
    alloc_interior: u16,
) {
    let tracked = toast.tracked_items();
    let alloc_body = usize::from(alloc_interior);

    // Reserve a line for the action hint if applicable.
    let has_action = toast.action_path().is_some() && alloc_body >= 2;
    let lines_for_body = if has_action {
        alloc_body.saturating_sub(1)
    } else {
        alloc_body
    };

    let lines: Vec<Line<'_>> = if tracked.is_empty() {
        body_lines_plain(toast, body_style, lines_for_body)
    } else {
        body_lines_tracked(
            tracked,
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
                "⏎ open",
                Style::default()
                    .fg(Color::DarkGray)
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

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
use crate::tui::constants::TOAST_GAP;
use crate::tui::constants::TOAST_WIDTH;
use crate::tui::types::ToastHitbox;

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

pub fn render_toasts(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView<'_>],
    pane_focused: bool,
    focused_toast_id: Option<u64>,
) -> Vec<ToastHitbox> {
    if toasts.is_empty() {
        return Vec::new();
    }

    let width = TOAST_WIDTH.min(area.width);

    // Stack toasts from the bottom of the area upward. Each toast may
    // have a different height due to entrance/exit animation.
    let mut hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.y.saturating_add(area.height);
    for toast in toasts.iter().rev() {
        let card_height = toast.visible_lines();
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
        let close_rect = render_toast_card(frame, card, toast, pane_focused, focused_toast_id);
        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
    }

    hitboxes.reverse();
    hitboxes
}

/// Render a single toast card and return the close-button rect for hit-testing.
fn render_toast_card(
    frame: &mut Frame,
    card: Rect,
    toast: &ToastView<'_>,
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
    render_toast_body(frame, toast, body_style, inner);

    close_rect
}

fn render_toast_body(frame: &mut Frame, toast: &ToastView<'_>, body_style: Style, body_area: Rect) {
    if toast.action_path().is_some() && body_area.height >= 2 {
        let text_area = Rect {
            height: body_area.height.saturating_sub(1),
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(toast.body())
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
            Paragraph::new(toast.body())
                .style(body_style)
                .wrap(Wrap { trim: false }),
            body_area,
        );
    }
}

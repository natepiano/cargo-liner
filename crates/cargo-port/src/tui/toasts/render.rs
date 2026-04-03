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

use super::manager::ToastView;
use crate::tui::constants::TOAST_GAP;
use crate::tui::constants::TOAST_HEIGHT;
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
    let step = TOAST_HEIGHT + TOAST_GAP;
    let max_cards = usize::from((area.height + TOAST_GAP) / step).max(1);
    let visible = if toasts.len() > max_cards {
        &toasts[toasts.len() - max_cards..]
    } else {
        toasts
    };

    let mut hitboxes = Vec::with_capacity(visible.len());
    for (stack_index, toast) in visible.iter().rev().enumerate() {
        let offset = u16::try_from(stack_index).unwrap_or(u16::MAX);
        let y = area
            .y
            .saturating_add(area.height)
            .saturating_sub(TOAST_HEIGHT)
            .saturating_sub(offset.saturating_mul(step));
        let x = area.x + area.width.saturating_sub(width);
        let card = Rect {
            x,
            y,
            width,
            height: TOAST_HEIGHT,
        };

        frame.render_widget(Clear, card);

        let focused = pane_focused && focused_toast_id == Some(toast.id());
        let border_style = if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(card);
        frame.render_widget(block, card);

        let close_text = "[x]";
        let close_width = u16::try_from(close_text.len()).unwrap_or(u16::MAX);
        let close_rect = Rect {
            x:      card.x + card.width.saturating_sub(close_width + 2),
            y:      card.y,
            width:  close_width + 1,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                close_text,
                border_style.add_modifier(Modifier::BOLD),
            ))),
            close_rect,
        );

        let title_width = usize::from(inner.width.saturating_sub(close_width + 1));
        let title = truncate(toast.title(), title_width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                title,
                border_style.add_modifier(Modifier::BOLD),
            ))),
            Rect {
                x:      inner.x,
                y:      inner.y,
                width:  inner.width.saturating_sub(close_width + 1),
                height: 1,
            },
        );
        frame.render_widget(
            Paragraph::new(toast.body()).wrap(Wrap { trim: false }),
            Rect {
                x:      inner.x,
                y:      inner.y.saturating_add(1),
                width:  inner.width,
                height: inner.height.saturating_sub(1),
            },
        );

        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
    }

    hitboxes.reverse();
    hitboxes
}

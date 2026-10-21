//! Read the foreground and background cells produced for every configured ink.

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::tests;
use super::*;
use crate::theme;

fn assert_drawn_ink(style: Style, ground: Color, description: &str) {
    let area = Rect::new(0, 0, 8, 1);
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(ground));
    Paragraph::new("reading")
        .style(style)
        .render(area, &mut buffer);
    for x in 0..7 {
        let cell = &buffer[(x, 0)];
        let ratio = tests::contrast_ratio(cell.fg, cell.bg);
        assert!(
            ratio >= MINIMUM_CONTRAST,
            "{description}: {cell:?} is {ratio}:1"
        );
    }
}

#[test]
fn configured_role_and_account_ink_is_readable_on_every_drawn_pane_ground() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        tui_pane::set_focused_pane_tint(true);
        for transparency in [false, true] {
            tui_pane::set_transparent_background(transparency);
            for color in FooterColor::ALL {
                let tint = FooterTint::Footer(color);
                for focused in [false, true] {
                    let ground = tui_pane::pane_background(focused);
                    for style in [
                        tint.style(Style::default()),
                        tint.subdued_style(Style::default()),
                    ] {
                        assert_drawn_ink(
                            style,
                            ground,
                            &format!("{} {color:?}", variant.id.as_str()),
                        );
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}

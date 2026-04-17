use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum PaneRule {
    Horizontal {
        area:        Rect,
        connector_x: Option<u16>,
    },
    Vertical {
        area: Rect,
    },
    Symbol {
        area:  Rect,
        glyph: char,
    },
}

pub(in super::super) fn render_rules(frame: &mut Frame, rules: &[PaneRule], style: Style) {
    for rule in rules {
        match *rule {
            PaneRule::Horizontal { area, connector_x } => {
                render_horizontal_rule(frame, area, style, connector_x);
            },
            PaneRule::Vertical { area } => render_vertical_rule(frame, area, style),
            PaneRule::Symbol { area, glyph } => render_symbol_rule(frame, area, style, glyph),
        }
    }
}

fn render_horizontal_rule(frame: &mut Frame, area: Rect, style: Style, connector_x: Option<u16>) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = (0..area.width)
        .map(|offset| {
            let x = area.x.saturating_add(offset);
            if offset == 0 {
                '├'
            } else if offset == area.width.saturating_sub(1) {
                '┤'
            } else if connector_x == Some(x) {
                '┬'
            } else {
                '─'
            }
        })
        .collect::<String>();
    frame.render_widget(Paragraph::new(Line::from(Span::styled(line, style))), area);
}

fn render_vertical_rule(frame: &mut Frame, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = (0..area.height)
        .map(|_| Line::from(Span::styled("│", style)))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_symbol_rule(frame: &mut Frame, area: Rect, style: Style, glyph: char) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(glyph.to_string(), style))),
        area,
    );
}

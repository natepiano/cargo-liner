use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

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

/// Optional title to embed near the left end of a horizontal rule.
#[derive(Clone, Copy)]
pub struct RuleTitle<'a> {
    pub text:  &'a str,
    pub style: Style,
}

pub(in super::super) fn render_rules(frame: &mut Frame, rules: &[PaneRule], style: Style) {
    for rule in rules {
        match *rule {
            PaneRule::Horizontal { area, connector_x } => {
                render_horizontal_rule(frame, area, style, None, connector_x);
            },
            PaneRule::Vertical { area } => render_vertical_rule(frame, area, style),
            PaneRule::Symbol { area, glyph } => render_symbol_rule(frame, area, style, glyph),
        }
    }
}

/// Render a horizontal rule with `├`/`┤` endcaps.
///
/// - `title`: when present, embeds `├─ Title ─...─┤`. Falls back to the plain form when the area is
///   too narrow to fit the title.
/// - `connector_x`: absolute x column that should render as `┬` instead of `─`, used when a
///   vertical pane border tees in from above. Only honored in the untitled form; a titled rule
///   ignores it since section headers don't intersect with vertical rules.
pub fn render_horizontal_rule(
    frame: &mut Frame,
    area: Rect,
    rule_style: Style,
    title: Option<RuleTitle<'_>>,
    connector_x: Option<u16>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = match title {
        Some(title) if fits_title(area.width, title.text) => {
            titled_line(area.width, title, rule_style)
        },
        _ => plain_line(area, rule_style, connector_x),
    };

    frame.render_widget(Paragraph::new(line), area);
}

fn fits_title(width: u16, title: &str) -> bool {
    // Layout budget: "├─ " + title + " " + "┤" = title.width() + 5.
    usize::from(width) >= title.width() + 5
}

fn titled_line(width: u16, title: RuleTitle<'_>, rule_style: Style) -> Line<'static> {
    const LEADING: &str = "├─ ";
    const TRAILING: &str = "┤";
    let total = usize::from(width);
    let dashes = total
        .saturating_sub(LEADING.width())
        .saturating_sub(title.text.width())
        .saturating_sub(1) // space between title and dashes
        .saturating_sub(TRAILING.width());
    let fill = "─".repeat(dashes);
    Line::from(vec![
        Span::styled(LEADING.to_string(), rule_style),
        Span::styled(title.text.to_string(), title.style),
        Span::styled(format!(" {fill}{TRAILING}"), rule_style),
    ])
}

fn plain_line(area: Rect, style: Style, connector_x: Option<u16>) -> Line<'static> {
    let glyphs: String = (0..area.width)
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
        .collect();
    Line::from(Span::styled(glyphs, style))
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

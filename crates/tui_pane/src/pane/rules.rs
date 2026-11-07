use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::PaneFrameLabel;

/// Optional title to embed near the left end of a horizontal rule.
#[derive(Clone, Copy)]
pub struct RuleTitle<'a> {
    /// Title text.
    pub text:  &'a str,
    /// Style applied to the title text.
    pub style: Style,
}

/// Where a titled rule writes its title.
///
/// The grid draws the rule and its border joins before it draws the label.
/// A narrow rule removes title padding before it cuts the title with `…`.
/// Return `None` only when no title cell fits between the borders.
#[must_use]
pub fn rule_title_label(
    area: Rect,
    title: RuleTitle<'_>,
    connector_x: Option<u16>,
) -> Option<PaneFrameLabel> {
    let (label_x, text) = match connector_x {
        Some(connector)
            if connector_in_area(area, connector)
                && fits_title_after_connector(area, connector, title.text) =>
        {
            (connector.saturating_add(1), format!(" {} ", title.text))
        },
        _ if fits_title(area.width, title.text) => {
            (area.x.saturating_add(2), format!(" {} ", title.text))
        },
        _ if usize::from(area.width) == title.text.width() + 4 => {
            (area.x.saturating_add(1), format!(" {} ", title.text))
        },
        _ if area.width > 2 => (
            area.x.saturating_add(1),
            narrow_rule_title(title.text, area.width - 2),
        ),
        _ => return None,
    };
    let width = u16::try_from(text.width()).unwrap_or(u16::MAX);
    Some(PaneFrameLabel {
        area: Rect::new(label_x, area.y, width, 1),
        text,
        style: title.style,
    })
}

fn narrow_rule_title(title: &str, width: u16) -> String {
    let prefix = if width > 1 { " " } else { "" };
    let budget = usize::from(width).saturating_sub(prefix.len());
    if title.width() <= budget {
        return format!("{prefix}{title}");
    }
    let mut text = prefix.to_string();
    let mut used = 0;
    for ch in title.chars() {
        let cells = ch.width().unwrap_or(0);
        if used + cells > budget.saturating_sub(1) {
            break;
        }
        text.push(ch);
        used += cells;
    }
    text.push('…');
    text
}

fn fits_title(width: u16, title: &str) -> bool {
    // `fits_title` budgets "├─ " + title + " " + "┤" = title.width() + 5.
    usize::from(width) >= title.width() + 5
}

const fn connector_in_area(area: Rect, connector_x: u16) -> bool {
    let first = area.x.saturating_add(1);
    let last = area.x.saturating_add(area.width).saturating_sub(2);
    connector_x >= first && connector_x <= last
}

fn fits_title_after_connector(area: Rect, connector_x: u16, title: &str) -> bool {
    // `fits_title_after_connector` budgets space right of the connector:
    // " Title ─...─┤" needs at least title.width() + 3 columns.
    let right_of_connector = area
        .x
        .saturating_add(area.width)
        .saturating_sub(connector_x.saturating_add(1));
    usize::from(right_of_connector) >= title.width() + 3
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    use super::RuleTitle;
    use super::rule_title_label;
    use crate::ColorMode;
    use crate::GridLines;
    use crate::PaneBorders;
    use crate::PaneFrame;
    use crate::default_pane_chrome;

    fn drawn_rule(width: u16) -> String {
        let area = Rect::new(0, 0, width, 3);
        let pane = PaneFrame::new(area);
        let rule = Rect::new(0, 1, width, 1);
        let mut grid = GridLines::new(area);
        grid.add(pane);
        grid.add_rule(pane, rule);
        if let Some(label) = rule_title_label(
            rule,
            RuleTitle {
                text:  "Worktrees (3)",
                style: Style::default(),
            },
            None,
        ) {
            grid.add_label(pane, label);
        }
        let mut buffer = Buffer::empty(area);
        grid.render(
            &mut buffer,
            default_pane_chrome(ColorMode::Colored),
            PaneBorders::Separate,
        );
        (0..width).map(|x| buffer[(x, 1)].symbol()).collect()
    }

    #[test]
    fn rules_keep_the_wide_title_prefix() {
        assert_eq!(drawn_rule(18), "├─ Worktrees (3) ┤");
    }

    #[test]
    fn rules_keep_the_title_with_four_extra_cells() {
        assert_eq!(drawn_rule(17), "├ Worktrees (3) ┤");
    }

    #[test]
    fn rules_keep_the_title_with_three_extra_cells() {
        assert_eq!(drawn_rule(16), "├ Worktrees (3)┤");
    }

    #[test]
    fn rules_shorten_the_title_inside_both_borders() {
        assert_eq!(drawn_rule(10), "├ Worktr…┤");
    }

    #[test]
    fn rules_use_the_last_title_cell_for_an_ellipsis() {
        assert_eq!(drawn_rule(3), "├…┤");
    }

    #[test]
    fn rules_keep_both_borders_when_no_title_cell_fits() {
        assert_eq!(drawn_rule(2), "├┤");
    }
}

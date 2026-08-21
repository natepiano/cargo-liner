use ratatui::layout::Rect;
use ratatui::style::Style;
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
/// A pane reports its rules to the grid rather than drawing them, so the
/// line itself — and every junction it makes with a border — comes out of
/// the grid pass. Only the title has to be placed, as a label written over
/// the line once every line is down: `─ Title ` reading `├─ Title ───┤`
/// across the finished rule, or `├──┬ Title ──┤` when a vertical rule tees
/// in at `connector_x`. `None` when the title does not fit and the rule
/// runs unbroken.
#[must_use]
pub fn rule_title_label(
    area: Rect,
    title: RuleTitle<'_>,
    connector_x: Option<u16>,
) -> Option<PaneFrameLabel> {
    // The label carries its own space on each side, so it starts on the
    // column before the title: right after the `┬` a vertical rule makes,
    // or two past the left end, where `├─` runs.
    let label_x = match connector_x {
        Some(connector)
            if connector_in_area(area, connector)
                && fits_title_after_connector(area, connector, title.text) =>
        {
            connector.saturating_add(1)
        },
        _ if fits_title(area.width, title.text) => area.x.saturating_add(2),
        _ => return None,
    };
    let width = u16::try_from(title.text.width().saturating_add(2)).unwrap_or(u16::MAX);
    Some(PaneFrameLabel {
        area:  Rect::new(label_x, area.y, width, 1),
        text:  format!(" {} ", title.text),
        style: title.style,
    })
}

fn fits_title(width: u16, title: &str) -> bool {
    // Layout budget: "├─ " + title + " " + "┤" = title.width() + 5.
    usize::from(width) >= title.width() + 5
}

const fn connector_in_area(area: Rect, connector_x: u16) -> bool {
    let first = area.x.saturating_add(1);
    let last = area.x.saturating_add(area.width).saturating_sub(2);
    connector_x >= first && connector_x <= last
}

fn fits_title_after_connector(area: Rect, connector_x: u16, title: &str) -> bool {
    // Space right of the connector: " Title ─...─┤" needs at least title.width() + 3 columns.
    let right_of_connector = area
        .x
        .saturating_add(area.width)
        .saturating_sub(connector_x.saturating_add(1));
    usize::from(right_of_connector) >= title.width() + 3
}

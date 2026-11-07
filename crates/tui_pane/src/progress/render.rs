//! Progress row vocabulary and panel rendering.

use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

use super::Percentage;
use super::ProgressState;

/// One labeled row of the startup panel.
#[derive(Debug, Clone)]
pub struct ProgressRow {
    /// Application-supplied row label.
    pub label:  &'static str,
    /// Current presentation state.
    pub state:  ProgressState,
    /// The item the row is currently working on (or about to), shown only
    /// once the row has been slow enough to warrant it. `ProgressDetail::Absent` keeps the row
    /// terse.
    pub detail: ProgressDetail,
}

/// Optional detail shown beside a progress row.
#[derive(Debug, Clone)]
pub enum ProgressDetail {
    /// Render only the label and state.
    Absent,
    /// Render this detail when enough width remains.
    Message(String),
}

/// Application-selected progress bar glyphs, width and row colors.
#[derive(Debug, Clone, Copy)]
pub struct ProgressPanelStyle {
    /// Number of glyphs in the progress bar.
    pub bar_width: usize,
    /// Glyph for completed portions of the bar.
    pub filled:    &'static str,
    /// Glyph for remaining portions of the bar.
    pub empty:     &'static str,
    /// RGB color at zero progress.
    pub active:    (u8, u8, u8),
    /// RGB color at complete progress.
    pub complete:  (u8, u8, u8),
    /// Color of an indeterminate row.
    pub waiting:   Color,
    /// Color of a failed row.
    pub failed:    Color,
}

/// Render each progress row with its foreground color.
///
/// Keep the label whole. Reduce the bar and state text before the label needs more space.
#[must_use]
pub fn progress_panel_body(
    rows: &[ProgressRow],
    body_width: usize,
    style: ProgressPanelStyle,
) -> (Vec<String>, Vec<Color>) {
    let label_width = rows.iter().map(|row| row.label.width()).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(rows.len());
    let mut colors = Vec::with_capacity(rows.len());
    for row in rows {
        let (percentage, suffix) = match row.state {
            ProgressState::Active(percentage) => (percentage, format!("{:>3}%", percentage.get())),
            ProgressState::CompleteHeld => (Percentage::full(), "100%".to_string()),
            ProgressState::Waiting => (Percentage::empty(), "waiting".to_string()),
            ProgressState::Failed => (Percentage::empty(), "failed".to_string()),
        };
        let fitted_label_width = label_width.min(body_width).max(row.label.width());
        let padding = fitted_label_width.saturating_sub(row.label.width());
        let mut line = format!("{}{}", row.label, " ".repeat(padding));
        let remaining = body_width.saturating_sub(fitted_label_width + 2);
        if remaining > 0 {
            let bar_width = style
                .bar_width
                .min(remaining.saturating_sub(suffix.width() + 1));
            line.push_str("  ");
            if bar_width > 0 {
                line.push_str(&progress_bar(
                    percentage,
                    ProgressPanelStyle { bar_width, ..style },
                ));
                line.push(' ');
            }
            line.push_str(&truncate_detail(
                &suffix,
                remaining.saturating_sub(bar_width + usize::from(bar_width > 0)),
            ));
        }
        if let ProgressDetail::Message(detail) = &row.detail {
            let used = line.width();
            let remaining = body_width.saturating_sub(used + 2);
            if remaining >= 4 {
                line.push_str("  ");
                line.push_str(&truncate_detail(detail, remaining));
            }
        }
        lines.push(line);
        colors.push(row_color(row.state, style));
    }
    (lines, colors)
}

/// A `style.bar_width`-wide fill bar: filled glyphs proportional to the
/// percentage, the remainder empty.
fn progress_bar(percentage: Percentage, style: ProgressPanelStyle) -> String {
    let filled = style.bar_width * usize::from(percentage.get()) / 100;
    let empty = style.bar_width - filled;
    format!(
        "{}{}",
        style.filled.repeat(filled),
        style.empty.repeat(empty),
    )
}

/// Interpolate the supplied RGB endpoints for determinate rows; otherwise
/// use the supplied complete, waiting or failed color.
fn row_color(state: ProgressState, style: ProgressPanelStyle) -> Color {
    match state {
        ProgressState::Active(percentage) => Color::Rgb(
            ramp_channel(percentage, style.active.0, style.complete.0),
            ramp_channel(percentage, style.active.1, style.complete.1),
            ramp_channel(percentage, style.active.2, style.complete.2),
        ),
        ProgressState::CompleteHeld => {
            Color::Rgb(style.complete.0, style.complete.1, style.complete.2)
        },
        ProgressState::Waiting => style.waiting,
        ProgressState::Failed => style.failed,
    }
}

/// Interpolate a color channel with the original integer rounding.
fn ramp_channel(percentage: Percentage, start: u8, end: u8) -> u8 {
    let distance = u16::from(start.abs_diff(end)) * u16::from(percentage.get()) / 100;
    let distance = u8::try_from(distance).unwrap_or(0);
    if start >= end {
        start.saturating_sub(distance)
    } else {
        start.saturating_add(distance)
    }
}

/// Truncate the detail to `width` columns, replacing the tail with `…` when
/// it doesn't fit.
fn truncate_detail(detail: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if detail.chars().count() <= width {
        return detail.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = detail.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const STARTUP_BAR_WIDTH: usize = 8;
    const STARTUP_BAR_FILLED: &str = "▓";
    const STARTUP_BAR_EMPTY: &str = "░";
    const STYLE: ProgressPanelStyle = ProgressPanelStyle {
        bar_width: STARTUP_BAR_WIDTH,
        filled:    STARTUP_BAR_FILLED,
        empty:     STARTUP_BAR_EMPTY,
        active:    (255, 255, 255),
        complete:  (0, 255, 0),
        waiting:   Color::Rgb(255, 255, 255),
        failed:    Color::Rgb(220, 90, 90),
    };
    fn startup_panel_body(rows: &[ProgressRow], width: usize) -> (Vec<String>, Vec<Color>) {
        progress_panel_body(rows, width, STYLE)
    }

    const WIDTH: usize = 58;

    fn full_bar() -> String { STARTUP_BAR_FILLED.repeat(STARTUP_BAR_WIDTH) }

    fn empty_bar() -> String { STARTUP_BAR_EMPTY.repeat(STARTUP_BAR_WIDTH) }

    fn row(label: &'static str, state: ProgressState) -> ProgressRow {
        ProgressRow {
            label,
            state,
            detail: ProgressDetail::Absent,
        }
    }

    #[test]
    fn narrow_progress_rows_draw_whole_labels_in_each_state() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Style;
        use ratatui::text::Line;
        use ratatui::widgets::Paragraph;
        use ratatui::widgets::Widget;

        use crate::ColorMode;

        let rows = [
            row(
                "Cargo metadata",
                ProgressState::Active(Percentage::from_fraction(1, 2)),
            ),
            row("Cargo metadata", ProgressState::Waiting),
            row("Cargo metadata", ProgressState::Failed),
            row("Cargo metadata", ProgressState::CompleteHeld),
        ];
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            for width in [14_u16, 18, 22, 27] {
                let (lines, colors) = startup_panel_body(&rows, usize::from(width));
                let text: Vec<_> = lines
                    .iter()
                    .zip(colors)
                    .map(|(line, color)| {
                        assert!(line.width() <= usize::from(width), "{width}: {line}");
                        Line::styled(line.clone(), Style::default().fg(color))
                    })
                    .collect();
                let area = Rect::new(0, 0, width, 4);
                let mut buffer = Buffer::empty(area);
                Paragraph::new(text).render(area, &mut buffer);
                mode.clear_colors(&mut buffer);
                for y in 0..4 {
                    let label: String = (0..14).map(|x| buffer[(x, y)].symbol()).collect();
                    assert_eq!(label, "Cargo metadata", "{mode:?}: {width}, row {y}");
                }
                if width == 27 {
                    for (line, suffix) in lines.iter().zip(["50%", "waiting", "failed", "100%"]) {
                        assert!(line.ends_with(suffix), "{line}");
                    }
                }
            }
        }
    }

    #[test]
    fn panel_body_renders_a_bar_and_percent_per_row() {
        let rows = [
            row("Disk usage", ProgressState::Active(Percentage::full())),
            row(
                "Cargo metadata",
                ProgressState::Active(Percentage::from_fraction(0, 4)),
            ),
        ];
        let (lines, colors) = startup_panel_body(&rows, WIDTH);
        assert_eq!(lines.len(), 2);
        assert_eq!(colors.len(), 2);
        assert!(lines[0].contains(&full_bar()) && lines[0].ends_with("100%"));
        assert!(lines[1].contains(&empty_bar()) && lines[1].ends_with("0%"));
        // `startup_panel_body` pads `ProgressRow::label` to the widest label so `bar_column` agrees
        // across `lines`.
        let bar_column = |line: &str| {
            line.find(STARTUP_BAR_FILLED)
                .or_else(|| line.find(STARTUP_BAR_EMPTY))
        };
        assert_eq!(bar_column(&lines[0]), bar_column(&lines[1]));
        // `colors[0]` is full green for 100% and `colors[1]` white for 0%.
        assert_eq!(colors[0], Color::Rgb(0, 255, 0));
        assert_eq!(colors[1], Color::Rgb(255, 255, 255));
    }

    #[test]
    fn ramp_is_linear_white_to_green() {
        assert_eq!(
            row_color(ProgressState::Active(Percentage::empty()), STYLE),
            Color::Rgb(255, 255, 255)
        );
        assert_eq!(
            row_color(ProgressState::Active(Percentage::full()), STYLE),
            Color::Rgb(0, 255, 0)
        );
        // `row_color` at 50% returns `Color::Rgb(128, 255, 128)`, halfway between white and green.
        let half = row_color(
            ProgressState::Active(Percentage::from_fraction(1, 2)),
            STYLE,
        );
        assert_eq!(half, Color::Rgb(128, 255, 128));
    }

    #[test]
    fn slow_row_shows_its_current_item_after_the_percent() {
        let mut r = row(
            "crates.io",
            ProgressState::Active(Percentage::from_fraction(1, 4)),
        );
        r.detail = ProgressDetail::Message("serde".to_string());
        let (lines, _) = startup_panel_body(std::slice::from_ref(&r), WIDTH);
        assert!(lines[0].ends_with("serde"));
    }

    #[test]
    fn complete_held_renders_a_full_green_bar() {
        let rows = [row("Disk usage", ProgressState::CompleteHeld)];
        let (lines, colors) = startup_panel_body(&rows, WIDTH);
        assert!(lines[0].contains(&full_bar()) && lines[0].ends_with("100%"));
        assert_eq!(colors[0], Color::Rgb(0, 255, 0));
    }

    #[test]
    fn empty_row_set_renders_empty_body() {
        let (lines, colors) = startup_panel_body(&[], WIDTH);
        assert!(lines.is_empty() && colors.is_empty());
    }

    #[test]
    fn overshoot_fraction_clamps_to_a_full_bar() {
        let rows = [row(
            "Disk usage",
            ProgressState::Active(Percentage::from_fraction(9, 4)),
        )];
        let (lines, _) = startup_panel_body(&rows, WIDTH);
        assert!(lines[0].contains(&full_bar()) && lines[0].ends_with("100%"));
    }

    #[test]
    fn waiting_row_renders_an_empty_bar_and_marker() {
        let rows = [row("GitHub repos", ProgressState::Waiting)];
        let (lines, colors) = startup_panel_body(&rows, WIDTH);
        assert!(lines[0].contains(&empty_bar()) && lines[0].ends_with("waiting"));
        assert_eq!(colors[0], Color::Rgb(255, 255, 255));
    }

    #[test]
    fn failed_row_renders_an_empty_bar_and_marker() {
        let rows = [row("GitHub repos", ProgressState::Failed)];
        let (lines, colors) = startup_panel_body(&rows, WIDTH);
        assert!(lines[0].contains(&empty_bar()) && lines[0].ends_with("failed"));
        assert_eq!(colors[0], Color::Rgb(220, 90, 90));
    }
}

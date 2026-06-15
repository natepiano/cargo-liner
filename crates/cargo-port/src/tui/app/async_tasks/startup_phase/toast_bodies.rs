use ratatui::style::Color;

use crate::tui::app::phase_state::Percentage;
use crate::tui::app::phase_state::ProgressRow;
use crate::tui::app::phase_state::ProgressState;
use crate::tui::constants::STARTUP_BAR_EMPTY;
use crate::tui::constants::STARTUP_BAR_FILLED;
use crate::tui::constants::STARTUP_BAR_WIDTH;

/// Render the multi-row startup panel body into one line per row plus a
/// matching per-line foreground color. Each line is a left-aligned label, a
/// fixed-width fill bar, a right-aligned percent, and — for a row that has
/// been slow enough — the item it is currently working on. The color ramps
/// linearly from white at 0% to full green at 100%.
pub(super) fn startup_panel_body(
    rows: &[ProgressRow],
    body_width: usize,
) -> (Vec<String>, Vec<Color>) {
    let label_width = rows
        .iter()
        .map(|row| row.label.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::with_capacity(rows.len());
    let mut colors = Vec::with_capacity(rows.len());
    for row in rows {
        // A determinate row shows a fill bar + percent; an indeterminate or
        // failed row shows an empty bar + a short word in the percent column.
        let (bar, suffix) = match row.state {
            ProgressState::Active(percentage) => (
                progress_bar(percentage),
                format!("{:>3}%", percentage.get()),
            ),
            ProgressState::CompleteHeld => (progress_bar(Percentage::full()), "100%".to_string()),
            ProgressState::Waiting => (progress_bar(Percentage::empty()), "waiting".to_string()),
            ProgressState::Failed => (progress_bar(Percentage::empty()), "failed".to_string()),
        };
        let mut line = format!("{label:<label_width$}  {bar} {suffix}", label = row.label);
        if let Some(detail) = &row.detail {
            let used = line.chars().count();
            let remaining = body_width.saturating_sub(used + 2);
            if remaining >= 4 {
                line.push_str("  ");
                line.push_str(&truncate_detail(detail, remaining));
            }
        }
        lines.push(line);
        colors.push(row_color(row.state));
    }
    (lines, colors)
}

/// A `STARTUP_BAR_WIDTH`-wide fill bar: filled glyphs proportional to the
/// percentage, the remainder empty.
fn progress_bar(percentage: Percentage) -> String {
    let filled = STARTUP_BAR_WIDTH * usize::from(percentage.get()) / 100;
    let empty = STARTUP_BAR_WIDTH - filled;
    format!(
        "{}{}",
        STARTUP_BAR_FILLED.repeat(filled),
        STARTUP_BAR_EMPTY.repeat(empty),
    )
}

/// The row's foreground color: a linear ramp from white (0%) to full green
/// (100%) for determinate rows, full green when held complete, white while
/// waiting, and a muted red on failure.
fn row_color(state: ProgressState) -> Color {
    match state {
        ProgressState::Active(percentage) => {
            let channel = ramp_channel(percentage);
            Color::Rgb(channel, 255, channel)
        },
        ProgressState::CompleteHeld => Color::Rgb(0, 255, 0),
        ProgressState::Waiting => Color::Rgb(255, 255, 255),
        ProgressState::Failed => Color::Rgb(220, 90, 90),
    }
}

/// The non-green channel value for a `0..=100%` ramp from white to green:
/// `255` at 0% down to `0` at 100%, integer math so no float cast is needed.
fn ramp_channel(percentage: Percentage) -> u8 {
    let filled = 255u16 * u16::from(percentage.get()) / 100;
    u8::try_from(255u16.saturating_sub(filled)).unwrap_or(0)
}

/// Truncate the detail to `width` columns, replacing the tail with `…` when
/// it doesn't fit.
fn truncate_detail(detail: &str, width: usize) -> String {
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

    const WIDTH: usize = 58;

    fn full_bar() -> String { STARTUP_BAR_FILLED.repeat(STARTUP_BAR_WIDTH) }

    fn empty_bar() -> String { STARTUP_BAR_EMPTY.repeat(STARTUP_BAR_WIDTH) }

    fn row(label: &'static str, state: ProgressState) -> ProgressRow {
        ProgressRow {
            label,
            state,
            detail: None,
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
        // Labels are padded to the widest so the bars line up.
        let bar_column = |line: &str| {
            line.find(STARTUP_BAR_FILLED)
                .or_else(|| line.find(STARTUP_BAR_EMPTY))
        };
        assert_eq!(bar_column(&lines[0]), bar_column(&lines[1]));
        // 0% is white, 100% is full green.
        assert_eq!(colors[0], Color::Rgb(0, 255, 0));
        assert_eq!(colors[1], Color::Rgb(255, 255, 255));
    }

    #[test]
    fn ramp_is_linear_white_to_green() {
        assert_eq!(
            row_color(ProgressState::Active(Percentage::empty())),
            Color::Rgb(255, 255, 255)
        );
        assert_eq!(
            row_color(ProgressState::Active(Percentage::full())),
            Color::Rgb(0, 255, 0)
        );
        // 50% sits halfway between white and green.
        let half = row_color(ProgressState::Active(Percentage::from_fraction(1, 2)));
        assert_eq!(half, Color::Rgb(128, 255, 128));
    }

    #[test]
    fn slow_row_shows_its_current_item_after_the_percent() {
        let mut r = row(
            "crates.io",
            ProgressState::Active(Percentage::from_fraction(1, 4)),
        );
        r.detail = Some("serde".to_string());
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

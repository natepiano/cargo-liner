use ratatui::style::Color;
use tui_pane::ProgressPanelStyle;
use tui_pane::ProgressRow;
use tui_pane::progress_panel_body;

use crate::tui::constants::STARTUP_BAR_EMPTY;
use crate::tui::constants::STARTUP_BAR_FILLED;
use crate::tui::constants::STARTUP_BAR_WIDTH;

/// Render startup rows using Cargo Port's existing bar and colors.
pub(super) fn startup_panel_body(
    rows: &[ProgressRow],
    body_width: usize,
) -> (Vec<String>, Vec<Color>) {
    progress_panel_body(
        rows,
        body_width,
        ProgressPanelStyle {
            bar_width: STARTUP_BAR_WIDTH,
            filled:    STARTUP_BAR_FILLED,
            empty:     STARTUP_BAR_EMPTY,
            active:    (255, 255, 255),
            complete:  (0, 255, 0),
            waiting:   Color::Rgb(255, 255, 255),
            failed:    Color::Rgb(220, 90, 90),
        },
    )
}

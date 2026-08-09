use ratatui::style::Style;
use tui_pane::PaneChrome;

/// Shared style constants for pane rendering.
pub(in crate::tui) struct RenderStyles {
    pub readonly_label: Style,
    pub chrome:         PaneChrome,
}

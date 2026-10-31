use ratatui::style::Style;

use super::PaneChrome;

/// Label style and chrome shared by pane renderers.
pub struct PaneRenderStyles {
    /// Style for labels that cannot be edited.
    pub readonly_label: Style,
    /// Border and title styling for the pane frame.
    pub chrome:         PaneChrome,
}

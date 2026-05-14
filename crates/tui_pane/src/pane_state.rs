use ratatui::style::Style;

use crate::ACTIVE_FOCUS_COLOR;
use crate::HOVER_FOCUS_COLOR;
use crate::REMEMBERED_FOCUS_COLOR;
use crate::Viewport;

/// Whether a pane currently has keyboard focus, last had it before the
/// user moved focus elsewhere, or is fully inactive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFocusState {
    /// Pane has keyboard focus.
    Active,
    /// Pane last had focus and is still highlighted.
    Remembered,
    /// Pane is unfocused.
    Inactive,
}

/// Selection state for a single row within a pane: focused-and-selected,
/// hovered, last-focused-and-selected, or none of those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneSelectionState {
    /// This row is the cursor row in a focused pane.
    Active,
    /// The mouse cursor is hovering this row.
    Hovered,
    /// This row was the cursor row when focus moved away.
    Remembered,
    /// Default row state.
    Unselected,
}

/// Resolve the selection state for `row` using `viewport.pos()` as the
/// cursor.
#[must_use]
pub const fn selection_state(
    viewport: &Viewport,
    row: usize,
    focus: PaneFocusState,
) -> PaneSelectionState {
    selection_state_for(viewport, viewport.pos(), row, focus)
}

/// [`selection_state`] variant that takes the cursor explicitly.
///
/// For callers whose cursor lives outside this viewport. The hovered-row
/// check still reads the viewport's own hovered field because hover is
/// always per pane.
#[must_use]
pub const fn selection_state_for(
    viewport: &Viewport,
    cursor: usize,
    row: usize,
    focus: PaneFocusState,
) -> PaneSelectionState {
    if row == cursor && matches!(focus, PaneFocusState::Active) {
        PaneSelectionState::Active
    } else if matches!(viewport.hovered(), Some(hovered_row) if hovered_row == row) {
        PaneSelectionState::Hovered
    } else if row == cursor && matches!(focus, PaneFocusState::Remembered) {
        PaneSelectionState::Remembered
    } else {
        PaneSelectionState::Unselected
    }
}

/// Background style for a cursor row given the owning pane's focus
/// state.
#[must_use]
pub fn selection_style(focus: PaneFocusState) -> Style {
    match focus {
        PaneFocusState::Active => Style::default().bg(ACTIVE_FOCUS_COLOR),
        PaneFocusState::Remembered => Style::default().bg(REMEMBERED_FOCUS_COLOR),
        PaneFocusState::Inactive => Style::default(),
    }
}

impl PaneSelectionState {
    /// Style overlay that visually communicates this selection state.
    #[must_use]
    pub fn overlay_style(self) -> Style {
        match self {
            Self::Active => selection_style(PaneFocusState::Active),
            Self::Hovered => Style::default().bg(HOVER_FOCUS_COLOR),
            Self::Remembered => selection_style(PaneFocusState::Remembered),
            Self::Unselected => Style::default(),
        }
    }

    /// Patch `style` with this selection state's overlay.
    #[must_use]
    pub fn patch(self, style: Style) -> Style { style.patch(self.overlay_style()) }
}

/// Format a 1-based scroll position as `"{pos+1} of {len}"`.
#[must_use]
pub fn scroll_indicator(pos: usize, len: usize) -> String { format!("{} of {len}", pos + 1) }

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;

    use super::PaneFocusState;
    use super::PaneSelectionState;
    use crate::ACTIVE_FOCUS_COLOR;
    use crate::HOVER_FOCUS_COLOR;
    use crate::REMEMBERED_FOCUS_COLOR;
    use crate::Viewport;

    #[test]
    fn active_selection_style_only_adds_background_and_emphasis() {
        let style = super::selection_style(PaneFocusState::Active);

        assert_eq!(style.fg, None);
        assert_eq!(style.bg, Some(ACTIVE_FOCUS_COLOR));
        assert_eq!(style.add_modifier, Modifier::default());
    }

    #[test]
    fn selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Red);
        let patched = PaneSelectionState::Active.patch(base);

        assert_eq!(patched.fg, Some(Color::Red));
        assert_eq!(patched.bg, Some(ACTIVE_FOCUS_COLOR));
        assert_eq!(patched.add_modifier, Modifier::default());
    }

    #[test]
    fn remembered_selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Green);
        let patched = PaneSelectionState::Remembered.patch(base);

        assert_eq!(patched.fg, Some(Color::Green));
        assert_eq!(patched.bg, Some(REMEMBERED_FOCUS_COLOR));
    }

    #[test]
    fn hovered_selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Blue);
        let patched = PaneSelectionState::Hovered.patch(base);

        assert_eq!(patched.fg, Some(Color::Blue));
        assert_eq!(patched.bg, Some(HOVER_FOCUS_COLOR));
    }

    #[test]
    fn selection_state_returns_hovered_for_non_selected_hovered_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_hovered(Some(2));

        assert_eq!(
            super::selection_state(&pane, 2, PaneFocusState::Inactive),
            PaneSelectionState::Hovered
        );
    }

    #[test]
    fn selection_state_prefers_cursor_over_hovered_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_pos(1);
        pane.set_hovered(Some(1));

        assert_eq!(
            super::selection_state(&pane, 1, PaneFocusState::Active),
            PaneSelectionState::Active
        );
    }

    #[test]
    fn selection_state_prefers_hover_for_inactive_selected_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_pos(0);
        pane.set_hovered(Some(0));

        assert_eq!(
            super::selection_state(&pane, 0, PaneFocusState::Inactive),
            PaneSelectionState::Hovered
        );
    }
}

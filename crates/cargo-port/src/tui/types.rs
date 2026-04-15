use ratatui::layout::Rect;
use ratatui::style::Style;

use super::constants::ACTIVE_FOCUS_COLOR;
use super::constants::REMEMBERED_FOCUS_COLOR;
use super::interaction::UiHitbox;

/// A bounded cursor for scrollable lists. Replaces raw `usize` index + manual
/// bounds checking with a single type that enforces invariants.
#[derive(Default, Clone)]
pub(super) struct ScrollState {
    pos: usize,
}

impl ScrollState {
    pub const fn pos(&self) -> usize { self.pos }

    pub const fn set(&mut self, pos: usize) { self.pos = pos; }

    pub const fn up(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    pub const fn down(&mut self, len: usize) {
        if len > 0 && self.pos < len - 1 {
            self.pos += 1;
        }
    }

    pub const fn jump_home(&mut self) { self.pos = 0; }

    pub const fn jump_end(&mut self, len: usize) { self.pos = len.saturating_sub(1); }

    /// Clamp position to `0..len`. Useful after the backing list shrinks.
    pub const fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.pos = 0;
        } else if self.pos >= len {
            self.pos = len - 1;
        }
    }
}

/// Per-pane state shared by every scrollable panel in the TUI.
///
/// Each pane owns its cursor, knows its row count, and stores the screen
/// region it occupies so navigation and mouse hit-testing are self-contained.
#[derive(Default, Clone)]
pub(super) struct Pane {
    cursor:        ScrollState,
    len:           usize,
    content_area:  Rect,
    scroll_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneFocusState {
    Active,
    Remembered,
    Inactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneSelectionState {
    Active,
    Remembered,
    Unselected,
}

impl Pane {
    pub const fn new() -> Self {
        Self {
            cursor:        ScrollState { pos: 0 },
            len:           0,
            content_area:  Rect::new(0, 0, 0, 0),
            scroll_offset: 0,
        }
    }

    // -- navigation (pane knows its own len) --

    pub const fn up(&mut self) { self.cursor.up(); }

    pub const fn down(&mut self) { self.cursor.down(self.len); }

    pub const fn home(&mut self) { self.cursor.jump_home(); }

    pub const fn end(&mut self) { self.cursor.jump_end(self.len); }

    // -- position --

    pub const fn pos(&self) -> usize { self.cursor.pos() }

    pub const fn set_pos(&mut self, pos: usize) { self.cursor.set(pos); }

    // -- length (auto-clamps cursor) --

    pub const fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor.clamp(len);
    }

    // -- layout --

    pub const fn set_content_area(&mut self, area: Rect) { self.content_area = area; }

    pub const fn set_scroll_offset(&mut self, offset: usize) { self.scroll_offset = offset; }

    pub const fn content_area(&self) -> Rect { self.content_area }

    pub const fn scroll_offset(&self) -> usize { self.scroll_offset }

    pub const fn len(&self) -> usize { self.len }

    pub const fn selection_state(&self, row: usize, focus: PaneFocusState) -> PaneSelectionState {
        if row == self.pos() {
            match focus {
                PaneFocusState::Active => PaneSelectionState::Active,
                PaneFocusState::Remembered => PaneSelectionState::Remembered,
                PaneFocusState::Inactive => PaneSelectionState::Unselected,
            }
        } else {
            PaneSelectionState::Unselected
        }
    }

    pub fn selection_style(focus: PaneFocusState) -> Style {
        match focus {
            PaneFocusState::Active => Style::default().bg(ACTIVE_FOCUS_COLOR),
            PaneFocusState::Remembered => Style::default().bg(REMEMBERED_FOCUS_COLOR),
            PaneFocusState::Inactive => Style::default(),
        }
    }
}

impl PaneSelectionState {
    pub fn overlay_style(self) -> Style {
        match self {
            Self::Active => Pane::selection_style(PaneFocusState::Active),
            Self::Remembered => Pane::selection_style(PaneFocusState::Remembered),
            Self::Unselected => Style::default(),
        }
    }

    pub fn patch(self, style: Style) -> Style { style.patch(self.overlay_style()) }
}

/// Format a 1-based scroll position as `"{pos+1} of {len}"`.
pub(super) fn scroll_indicator(pos: usize, len: usize) -> String { format!("{} of {len}", pos + 1) }

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub(super) enum PaneId {
    #[default]
    ProjectList,
    Package,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
    Toasts,
    Search,
    Settings,
    Finder,
    Keymap,
}

impl PaneId {
    pub const fn is_overlay(self) -> bool {
        matches!(self, Self::Search | Self::Settings | Self::Finder)
    }
}

/// Cached layout rectangles from the last render frame, used for mouse
/// hit-testing in the event handler.
#[derive(Default)]
pub(super) struct LayoutCache {
    pub project_list:        Rect,
    pub project_list_offset: usize,
    pub detail_columns:      Vec<Rect>,
    pub ui_hitboxes:         Vec<UiHitbox>,
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;

    use super::PaneFocusState;
    use super::PaneSelectionState;

    #[test]
    fn active_selection_style_only_adds_background_and_emphasis() {
        let style = super::Pane::selection_style(PaneFocusState::Active);

        assert_eq!(style.fg, None);
        assert_eq!(style.bg, Some(super::ACTIVE_FOCUS_COLOR));
        assert_eq!(style.add_modifier, Modifier::default());
    }

    #[test]
    fn selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Red);
        let patched = PaneSelectionState::Active.patch(base);

        assert_eq!(patched.fg, Some(Color::Red));
        assert_eq!(patched.bg, Some(super::ACTIVE_FOCUS_COLOR));
        assert_eq!(patched.add_modifier, Modifier::default());
    }

    #[test]
    fn remembered_selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Green);
        let patched = PaneSelectionState::Remembered.patch(base);

        assert_eq!(patched.fg, Some(Color::Green));
        assert_eq!(patched.bg, Some(super::REMEMBERED_FOCUS_COLOR));
    }
}

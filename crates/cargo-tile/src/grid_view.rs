//! Own the expanded cell's keys and scrolling. `tui_pane::FullScreenView` owns its motion.

use std::time::Instant;

use crossterm::event::KeyCode;
use tui_pane::FullScreenView;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::ScrollDirection;

use crate::census::InvocationId;

/// Classify a cell key before the first paint resolves its viewport.
enum CellKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Focus,
    Global,
}

impl From<KeyCode> for CellKey {
    fn from(code: KeyCode) -> Self {
        match code {
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::PageUp => Self::PageUp,
            KeyCode::PageDown => Self::PageDown,
            KeyCode::Home => Self::Home,
            KeyCode::End => Self::End,
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => Self::Focus,
            _ => Self::Global,
        }
    }
}

/// The expanded cell's scroll position and the row measures of its last draw.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CellScroll {
    pub(crate) scroll: usize,
    pub(crate) rows:   usize,
    pub(crate) shown:  usize,
}

impl CellScroll {
    fn key(&mut self, code: KeyCode) -> KeyOutcome {
        let last = self.rows.saturating_sub(self.shown);
        self.scroll = match CellKey::from(code) {
            CellKey::Up => self.scroll.saturating_sub(1),
            CellKey::Down => self.scroll.saturating_add(1).min(last),
            CellKey::PageUp => self.scroll.saturating_sub(self.shown),
            CellKey::PageDown => self.scroll.saturating_add(self.shown).min(last),
            CellKey::Home => 0,
            CellKey::End => last,
            CellKey::Focus => {
                return KeyOutcome::Consumed;
            },
            CellKey::Global => return KeyOutcome::Unhandled,
        };
        KeyOutcome::Consumed
    }
}

/// The grid, or the expanded cell with its scroll position.
pub(crate) type GridView = FullScreenView<InvocationId, CellScroll>;

/// Grid, view, and row motion use display time.
pub(super) struct DisplayClock {
    displayed: Instant,
    sampled:   Instant,
}

impl DisplayClock {
    pub(super) const fn new(now: Instant) -> Self {
        Self {
            displayed: now,
            sampled:   now,
        }
    }

    pub(super) const fn held(&self) -> Instant { self.displayed }

    pub(super) fn at(&self, now: Instant) -> Instant {
        self.displayed + now.saturating_duration_since(self.sampled)
    }

    pub(super) fn sample(&mut self, now: Instant) -> Instant {
        self.displayed = self.at(now);
        self.sampled = self.sampled.max(now);
        self.displayed
    }

    pub(super) const fn resume(&mut self, now: Instant) { self.sampled = now; }
}

/// How preparation samples body motion for this frame.
#[derive(Clone, Copy)]
pub(crate) enum DisplayFrameTime {
    Live,
    Paused(Instant),
}

/// Scroll the expanded cell one row, as the arrow key does.
pub(super) fn scroll(view: &mut GridView, direction: ScrollDirection) {
    let code = match direction {
        ScrollDirection::Up => KeyCode::Up,
        ScrollDirection::Down => KeyCode::Down,
    };
    if let Some(cell) = view.policy_mut() {
        cell.key(code);
    }
}

/// Esc collapses the cell, and the arrow and page keys scroll it.
pub(super) fn key(view: &mut GridView, bind: &KeyBind, display_time: Instant) -> KeyOutcome {
    if matches!(view, GridView::Grid) {
        return KeyOutcome::Unhandled;
    }
    if !bind.mods.is_empty() {
        return match bind.code {
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Tab
            | KeyCode::BackTab => KeyOutcome::Consumed,
            _ => KeyOutcome::Unhandled,
        };
    }
    if bind.code == KeyCode::Esc {
        view.collapse(display_time);
        return KeyOutcome::Consumed;
    }
    view.policy_mut().map_or_else(
        || match CellKey::from(bind.code) {
            CellKey::Global => KeyOutcome::Unhandled,
            _ => KeyOutcome::Consumed,
        },
        |cell| cell.key(bind.code),
    )
}

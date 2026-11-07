//! Own the expanded cell identity, scrolling and motion clock.

use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Repaint;
use tui_pane::ScrollDirection;

use crate::tiles::TileContent;

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

/// A resolved cell owns its allocation, scroll and sampled motion clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CellViewport {
    pub(crate) content:   TileContent,
    pub(crate) origin:    Rect,
    pub(crate) displayed: Rect,
    pub(crate) sampled:   Instant,
    pub(crate) scroll:    usize,
    pub(crate) rows:      usize,
    pub(crate) shown:     usize,
}

impl CellViewport {
    pub(crate) const fn new(content: TileContent, area: Rect, sampled: Instant) -> Self {
        Self {
            content,
            origin: area,
            displayed: area,
            sampled,
            scroll: 0,
            rows: 0,
            shown: 0,
        }
    }

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

/// Only the first frame must resolve the focused cell.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum GridView {
    #[default]
    Grid,
    Opening {
        sampled: Instant,
        started: Instant,
    },
    Expanding {
        cell:    CellViewport,
        started: Instant,
    },
    Expanded {
        cell: CellViewport,
    },
    Collapsing {
        cell:    CellViewport,
        started: Instant,
    },
}

impl GridView {
    pub(crate) const fn advance(&mut self, now: Instant) -> Repaint {
        match self {
            Self::Opening { sampled, .. } => {
                *sampled = now;
                Repaint::Needed
            },
            Self::Expanding { cell, .. } | Self::Collapsing { cell, .. } => {
                cell.sampled = now;
                Repaint::Needed
            },
            Self::Grid | Self::Expanded { .. } => Repaint::NotNeeded,
        }
    }

    pub(crate) fn expand(&mut self, now: Instant) {
        match self.clone() {
            Self::Grid => {
                *self = Self::Opening {
                    sampled: now,
                    started: now,
                }
            },
            Self::Collapsing { mut cell, .. } => {
                cell.origin = cell.displayed;
                cell.sampled = now;
                *self = Self::Expanding { cell, started: now };
            },
            Self::Opening { .. } | Self::Expanding { .. } | Self::Expanded { .. } => {},
        }
    }

    fn collapse(&mut self, now: Instant) {
        match self.clone() {
            Self::Opening { .. } => *self = Self::Grid,
            Self::Expanding { mut cell, .. } | Self::Expanded { mut cell } => {
                cell.origin = cell.displayed;
                cell.sampled = now;
                *self = Self::Collapsing { cell, started: now };
            },
            Self::Grid | Self::Collapsing { .. } => {},
        }
    }

    pub(crate) fn scroll(&mut self, direction: ScrollDirection) {
        let code = match direction {
            ScrollDirection::Up => KeyCode::Up,
            ScrollDirection::Down => KeyCode::Down,
        };
        self.key(&KeyBind::from(code));
    }

    pub(crate) fn key(&mut self, bind: &KeyBind) -> KeyOutcome {
        if matches!(self, Self::Grid) {
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
            self.collapse(Instant::now());
            return KeyOutcome::Consumed;
        }
        match self {
            Self::Expanding { cell, .. }
            | Self::Expanded { cell }
            | Self::Collapsing { cell, .. } => cell.key(bind.code),
            Self::Opening { .. } => match CellKey::from(bind.code) {
                CellKey::Global => KeyOutcome::Unhandled,
                _ => KeyOutcome::Consumed,
            },
            Self::Grid => KeyOutcome::Unhandled,
        }
    }
}

use ratatui::layout::Position;
use tui_pane::Hittable;
use tui_pane::KeyBind;
use tui_pane::NavAction;
use tui_pane::Viewport;

use crate::sccache::StatsResult;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SccacheTarget {
    pub(super) label: String,
    pub(super) value: String,
}

/// A copyable value and the rendered line that owns it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SccacheSelectableLine {
    pub(super) line_index: usize,
    pub(super) target:     SccacheTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SccacheStatus {
    Loading,
    Ready { lines: Vec<String> },
    Failed { lines: Vec<String> },
}

pub(in crate::tui) struct SccachePane {
    status:                   SccacheStatus,
    request_id:               u64,
    selectable_lines:         Vec<SccacheSelectableLine>,
    pub viewport:             Viewport,
    pub(super) pending_chord: Vec<KeyBind>,
}

impl SccachePane {
    pub(in crate::tui) const fn new() -> Self {
        Self {
            status:           SccacheStatus::Loading,
            request_id:       0,
            selectable_lines: Vec::new(),
            viewport:         Viewport::new(),
            pending_chord:    Vec::new(),
        }
    }

    pub(super) const fn status(&self) -> &SccacheStatus { &self.status }

    pub(in crate::tui) const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub(super) fn set_selectable_lines(&mut self, lines: Vec<SccacheSelectableLine>) {
        self.viewport.set_len(lines.len());
        self.selectable_lines = lines;
    }

    pub(in crate::tui) fn row_at(&self, pos: Position) -> Option<usize> {
        let inner = self.viewport.content_area();
        if inner.width == 0 || inner.height == 0 || !inner.contains(pos) {
            return None;
        }
        let line_index = usize::from(pos.y.saturating_sub(inner.y)) + self.viewport.scroll_offset();
        self.selectable_lines
            .iter()
            .position(|line| line.line_index == line_index)
    }

    pub(super) fn selected_line(&self) -> Option<&SccacheSelectableLine> {
        self.selectable_lines.get(self.viewport.pos())
    }

    pub(super) fn selected_copy_value(&self) -> Option<&SccacheTarget> {
        self.selected_line().map(|line| &line.target)
    }

    pub(in crate::tui) fn start_loading(&mut self) -> u64 {
        self.pending_chord.clear();
        self.request_id = self.request_id.saturating_add(1);
        self.status = SccacheStatus::Loading;
        self.viewport.home();
        self.request_id
    }

    pub(in crate::tui) fn apply_result(&mut self, request_id: u64, result: StatsResult) {
        if self.request_id != request_id {
            return;
        }
        self.status = match result {
            StatsResult::Ready(lines) => SccacheStatus::Ready { lines },
            StatsResult::Failed(lines) => SccacheStatus::Failed { lines },
        };
        self.viewport.home();
    }

    pub(in crate::tui) fn navigate(&mut self, action: NavAction) {
        match action {
            NavAction::Up => self.viewport.up(),
            NavAction::Down => self.viewport.down(),
            NavAction::Home => self.viewport.home(),
            NavAction::End => {
                let last = self.viewport.len().saturating_sub(1);
                self.viewport.set_pos(last);
            },
            NavAction::PageUp => self.viewport.page_up(),
            NavAction::PageDown => self.viewport.page_down(),
            NavAction::HalfPageUp => self.viewport.half_page_up(),
            NavAction::HalfPageDown => self.viewport.half_page_down(),
            NavAction::Left | NavAction::Right => {},
        }
    }
}

impl Hittable<HoverTarget> for SccachePane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        self.row_at(pos).map(|row| HoverTarget::PaneRow {
            pane: PaneId::Sccache,
            row,
        })
    }
}

impl Default for SccachePane {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_another_request_retries_and_ignores_the_previous_result() {
        let mut pane = SccachePane::new();
        let previous_request = pane.start_loading();
        pane.apply_result(
            previous_request,
            StatsResult::Ready(vec!["previous".to_string()]),
        );

        let current_request = pane.start_loading();
        assert_eq!(pane.status(), &SccacheStatus::Loading);

        pane.apply_result(
            previous_request,
            StatsResult::Failed(vec!["late failure".to_string()]),
        );
        assert_eq!(pane.status(), &SccacheStatus::Loading);

        pane.apply_result(
            current_request,
            StatsResult::Ready(vec!["current".to_string()]),
        );
        assert_eq!(
            pane.status(),
            &SccacheStatus::Ready {
                lines: vec!["current".to_string()],
            }
        );
    }
}

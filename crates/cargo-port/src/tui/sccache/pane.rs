use crossterm::event::KeyCode;
use ratatui::layout::Position;
use tui_pane::Hittable;
use tui_pane::Viewport;

use crate::sccache::StatsResult;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SccacheTarget {
    pub(super) label: String,
    pub(super) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SccacheStatus {
    Loading { source: String },
    NotConfigured,
    Ready { source: String, lines: Vec<String> },
    Failed { source: String, lines: Vec<String> },
}

pub struct SccachePane {
    status:            SccacheStatus,
    request_id:        u64,
    line_targets:      Vec<Option<usize>>,
    selectable_values: Vec<SccacheTarget>,
    pub viewport:      Viewport,
}

impl SccachePane {
    pub const fn new() -> Self {
        Self {
            status:            SccacheStatus::NotConfigured,
            request_id:        0,
            line_targets:      Vec::new(),
            selectable_values: Vec::new(),
            viewport:          Viewport::new(),
        }
    }

    pub(super) const fn status(&self) -> &SccacheStatus { &self.status }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub(super) fn set_line_targets(
        &mut self,
        line_targets: Vec<Option<usize>>,
        selectable_values: Vec<SccacheTarget>,
    ) {
        self.line_targets = line_targets;
        self.selectable_values = selectable_values;
    }

    pub fn row_at(&self, pos: Position) -> Option<usize> {
        let inner = self.viewport.content_area();
        if inner.width == 0 || inner.height == 0 || !inner.contains(pos) {
            return None;
        }
        let line_index = usize::from(pos.y.saturating_sub(inner.y)) + self.viewport.scroll_offset();
        self.line_targets.get(line_index).copied().flatten()
    }

    pub(super) fn line_for_selection(&self, selection: usize) -> Option<usize> {
        self.line_targets
            .iter()
            .position(|target| target.is_some_and(|target| target == selection))
    }

    pub(super) fn selected_copy_value(&self) -> Option<&SccacheTarget> {
        self.selectable_values.get(self.viewport.pos())
    }

    pub fn show_not_configured(&mut self) {
        self.status = SccacheStatus::NotConfigured;
        self.viewport.home();
    }

    pub fn start_loading(&mut self, source: String) -> u64 {
        self.request_id = self.request_id.saturating_add(1);
        self.status = SccacheStatus::Loading { source };
        self.viewport.home();
        self.request_id
    }

    pub fn apply_result(&mut self, request_id: u64, result: StatsResult) {
        if self.request_id != request_id {
            return;
        }
        let source = match &self.status {
            SccacheStatus::Loading { source }
            | SccacheStatus::Ready { source, .. }
            | SccacheStatus::Failed { source, .. } => source.clone(),
            SccacheStatus::NotConfigured => return,
        };
        self.status = match result {
            StatsResult::Ready(lines) => SccacheStatus::Ready { source, lines },
            StatsResult::Failed(lines) => SccacheStatus::Failed { source, lines },
        };
        self.viewport.home();
    }

    pub fn handle_navigation_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.viewport.up(),
            KeyCode::Down => self.viewport.down(),
            KeyCode::Home => self.viewport.home(),
            KeyCode::End => {
                let last = self.viewport.len().saturating_sub(1);
                self.viewport.set_pos(last);
            },
            KeyCode::PageUp => self.viewport.page_up(),
            KeyCode::PageDown => self.viewport.page_down(),
            _ => {},
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

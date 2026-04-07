use super::types::App;
use super::types::BottomPanel;
use super::types::ExitMode;
use super::types::FinderMode;
use super::types::KeymapMode;
use super::types::SearchMode;
use super::types::SelectionSync;
use super::types::SettingsMode;
use crate::tui::shortcuts::InputContext;
use crate::tui::types::PaneId;

impl App {
    const TAB_ORDER: [PaneId; 7] = [
        PaneId::ProjectList,
        PaneId::Package,
        PaneId::Git,
        PaneId::Targets,
        PaneId::Lints,
        PaneId::CiRuns,
        PaneId::Toasts,
    ];

    pub const fn is_searching(&self) -> bool { self.ui_modes.search.is_active() }

    pub const fn is_finder_open(&self) -> bool { self.ui_modes.finder.is_visible() }

    pub const fn is_settings_open(&self) -> bool { self.ui_modes.settings.is_visible() }

    pub const fn is_settings_editing(&self) -> bool { self.ui_modes.settings.is_editing() }

    pub const fn is_scan_complete(&self) -> bool { self.scan.phase.is_complete() }

    pub const fn should_quit(&self) -> bool { self.ui_modes.exit.should_quit() }

    pub const fn should_restart(&self) -> bool { self.ui_modes.exit.should_restart() }

    pub const fn selection_changed(&self) -> bool { self.selection.is_changed() }

    pub const fn mark_selection_changed(&mut self) { self.selection = SelectionSync::Changed; }

    pub const fn clear_selection_changed(&mut self) { self.selection = SelectionSync::Stable; }

    pub const fn request_quit(&mut self) { self.ui_modes.exit = ExitMode::Quit; }

    pub const fn request_restart(&mut self) { self.ui_modes.exit = ExitMode::Restart; }

    pub const fn open_finder(&mut self) { self.ui_modes.finder = FinderMode::Visible; }

    pub const fn close_finder(&mut self) { self.ui_modes.finder = FinderMode::Hidden; }

    pub const fn end_search(&mut self) { self.ui_modes.search = SearchMode::Inactive; }

    pub const fn open_settings(&mut self) { self.ui_modes.settings = SettingsMode::Browsing; }

    pub const fn close_settings(&mut self) { self.ui_modes.settings = SettingsMode::Hidden; }

    pub const fn is_keymap_open(&self) -> bool { self.ui_modes.keymap.is_visible() }

    pub const fn open_keymap(&mut self) { self.ui_modes.keymap = KeymapMode::Browsing; }

    pub fn close_keymap(&mut self) {
        self.ui_modes.keymap = KeymapMode::Hidden;
        self.keymap_conflict = None;
    }

    pub fn keymap_begin_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::AwaitingKey;
        self.keymap_conflict = None;
    }

    pub fn keymap_end_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::Browsing;
        self.keymap_conflict = None;
    }

    pub const fn begin_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Editing;
    }

    pub const fn end_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
    }

    pub const fn mark_terminal_dirty(&mut self) { self.dirty.terminal.mark_dirty(); }

    pub const fn clear_terminal_dirty(&mut self) { self.dirty.terminal.mark_clean(); }

    pub const fn terminal_is_dirty(&self) -> bool { self.dirty.terminal.is_dirty() }

    /// Derive the current input context from app state.
    pub const fn input_context(&self) -> InputContext {
        if self.ui_modes.keymap.is_awaiting_key() && self.keymap_conflict.is_some() {
            InputContext::KeymapConflict
        } else if self.ui_modes.keymap.is_awaiting_key() {
            InputContext::KeymapAwaiting
        } else if self.ui_modes.keymap.is_visible() {
            InputContext::Keymap
        } else if self.ui_modes.finder.is_visible() {
            InputContext::Finder
        } else if self.is_settings_editing() {
            InputContext::SettingsEditing
        } else if self.ui_modes.settings.is_visible() {
            InputContext::Settings
        } else if self.ui_modes.search.is_active() {
            InputContext::Searching
        } else {
            match self.focused_pane {
                PaneId::Package | PaneId::Git => InputContext::DetailFields,
                PaneId::Targets => InputContext::DetailTargets,
                PaneId::Lints => InputContext::Lints,
                PaneId::CiRuns => InputContext::CiRuns,
                PaneId::Toasts => InputContext::Toasts,
                PaneId::Search => InputContext::Searching,
                PaneId::Settings => InputContext::Settings,
                PaneId::Finder => InputContext::Finder,
                PaneId::Keymap | PaneId::ProjectList => InputContext::ProjectList,
            }
        }
    }

    pub fn is_focused(&self, pane: PaneId) -> bool { self.focused_pane == pane }

    pub fn base_focus(&self) -> PaneId {
        if self.focused_pane.is_overlay() {
            self.return_focus.unwrap_or(PaneId::ProjectList)
        } else {
            self.focused_pane
        }
    }

    pub fn focus_pane(&mut self, pane: PaneId) {
        self.focused_pane = pane;
        if !pane.is_overlay() {
            self.visited_panes.insert(pane);
            self.return_focus = None;
        }
    }

    pub fn open_overlay(&mut self, pane: PaneId) {
        if !pane.is_overlay() {
            self.focus_pane(pane);
            return;
        }
        self.return_focus = Some(self.base_focus());
        self.focused_pane = pane;
    }

    pub fn close_overlay(&mut self) {
        self.focused_pane = self.return_focus.unwrap_or(PaneId::ProjectList);
        self.return_focus = None;
    }

    pub fn tabbable_panes(&self) -> Vec<PaneId> {
        Self::TAB_ORDER
            .into_iter()
            .filter(|pane| match pane {
                PaneId::ProjectList => true,
                PaneId::Package => self.selected_project_path().is_some(),
                PaneId::Git => self.selected_project_path().is_some_and(|path| {
                    self.git_info
                        .get(path)
                        .is_some_and(|info| info.url.is_some())
                }),
                PaneId::Targets => self.cached_detail.as_ref().is_some_and(|c| {
                    c.info.is_binary || !c.info.examples.is_empty() || !c.info.benches.is_empty()
                }),
                PaneId::Lints => self.selected_project_path().is_some() && self.lint_enabled(),
                PaneId::CiRuns => self
                    .selected_project_path()
                    .is_some_and(|path| self.bottom_panel_available(path)),
                PaneId::Toasts => !self.active_toasts().is_empty(),
                PaneId::Search | PaneId::Settings | PaneId::Finder | PaneId::Keymap => false,
            })
            .collect()
    }

    pub fn focus_next_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.toast_pane.pos() + 1 < self.active_toasts().len() {
            self.toast_pane.down();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
        if next == PaneId::Toasts {
            self.toast_pane.home();
        }
    }

    pub fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.toast_pane.pos() > 0 {
            self.toast_pane.up();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
        if prev == PaneId::Toasts {
            self.toast_pane
                .set_pos(self.active_toasts().len().saturating_sub(1));
        }
    }

    pub fn reset_project_panes(&mut self) {
        self.package_pane.home();
        self.git_pane.home();
        self.targets_pane.home();
        self.ci_pane.home();
        self.lint_pane.home();
        self.toast_pane.home();
        self.visited_panes.remove(&PaneId::Package);
        self.visited_panes.remove(&PaneId::Git);
        self.visited_panes.remove(&PaneId::Targets);
        self.visited_panes.remove(&PaneId::CiRuns);
    }

    pub fn remembers_selection(&self, pane: PaneId) -> bool { self.visited_panes.contains(&pane) }

    pub const fn toggle_bottom_panel(&mut self) {
        self.bottom_panel = match self.bottom_panel {
            BottomPanel::CiRuns => BottomPanel::Lints,
            BottomPanel::Lints => BottomPanel::CiRuns,
        };
    }

    pub const fn showing_lints(&self) -> bool { matches!(self.bottom_panel, BottomPanel::Lints) }
}

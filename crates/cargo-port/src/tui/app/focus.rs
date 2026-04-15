use strum::IntoEnumIterator;

use super::App;
use super::types::ExitMode;
use super::types::FinderMode;
use super::types::KeymapMode;
use super::types::SearchMode;
use super::types::SelectionSync;
use super::types::SettingsMode;
use crate::tui::detail::TargetsData;
use crate::tui::settings::SettingOption;
use crate::tui::settings::SettingOption::IncludeDirs;
use crate::tui::shortcuts::InputContext;
use crate::tui::types::PaneFocusState;
use crate::tui::types::PaneId;
use crate::tui::types::PaneId::Settings;

impl App {
    const TAB_ORDER: [PaneId; 9] = [
        PaneId::ProjectList,
        PaneId::Package,
        PaneId::Targets,
        PaneId::Lang,
        PaneId::Git,
        PaneId::Lints,
        PaneId::CiRuns,
        PaneId::Output,
        PaneId::Toasts,
    ];

    pub(in super::super) const fn is_searching(&self) -> bool { self.ui_modes.search.is_active() }

    pub(in super::super) const fn is_finder_open(&self) -> bool {
        self.ui_modes.finder.is_visible()
    }

    pub(in super::super) const fn is_settings_open(&self) -> bool {
        self.ui_modes.settings.is_visible()
    }

    pub(in super::super) const fn is_settings_editing(&self) -> bool {
        self.ui_modes.settings.is_editing()
    }

    pub(in super::super) const fn is_scan_complete(&self) -> bool { self.scan.phase.is_complete() }

    pub(in super::super) const fn should_quit(&self) -> bool { self.ui_modes.exit.should_quit() }

    pub(in super::super) const fn should_restart(&self) -> bool {
        self.ui_modes.exit.should_restart()
    }

    pub(in super::super) const fn selection_changed(&self) -> bool { self.selection.is_changed() }

    pub(in super::super) const fn mark_selection_changed(&mut self) {
        self.selection = SelectionSync::Changed;
    }

    pub(in super::super) const fn clear_selection_changed(&mut self) {
        self.selection = SelectionSync::Stable;
    }

    pub(in super::super) const fn request_quit(&mut self) { self.ui_modes.exit = ExitMode::Quit; }

    pub(in super::super) const fn request_restart(&mut self) {
        self.ui_modes.exit = ExitMode::Restart;
    }

    pub(in super::super) const fn open_finder(&mut self) {
        self.ui_modes.finder = FinderMode::Visible;
    }

    pub(in super::super) const fn close_finder(&mut self) {
        self.ui_modes.finder = FinderMode::Hidden;
    }

    pub(in super::super) const fn end_search(&mut self) {
        self.ui_modes.search = SearchMode::Inactive;
    }

    pub(in super::super) const fn open_settings(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
    }

    pub(in super::super) fn close_settings(&mut self) {
        self.ui_modes.settings = SettingsMode::Hidden;
        self.inline_error = None;
    }

    /// Open the settings overlay and position the cursor on `IncludeDirs`
    /// when no include directories are configured.
    pub(in super::super) fn force_settings_if_unconfigured(&mut self) {
        if !self.current_config.tui.include_dirs.is_empty() {
            return;
        }
        self.open_overlay(Settings);
        self.open_settings();
        if let Some(idx) = SettingOption::iter().position(|s| s == IncludeDirs) {
            self.pane_manager.settings.set_pos(idx);
        }
        self.set_inline_error("Configure at least one include directory before continuing");
    }

    pub(in super::super) const fn is_keymap_open(&self) -> bool {
        self.ui_modes.keymap.is_visible()
    }

    pub(in super::super) const fn open_keymap(&mut self) {
        self.ui_modes.keymap = KeymapMode::Browsing;
    }

    pub(in super::super) fn close_keymap(&mut self) {
        self.ui_modes.keymap = KeymapMode::Hidden;
        self.inline_error = None;
    }

    pub(in super::super) fn keymap_begin_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::AwaitingKey;
        self.inline_error = None;
    }

    pub(in super::super) fn keymap_end_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::Browsing;
        self.inline_error = None;
    }

    pub(in super::super) fn begin_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Editing;
        self.inline_error = None;
    }

    pub(in super::super) fn end_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
        self.inline_error = None;
    }

    pub(in super::super) const fn mark_terminal_dirty(&mut self) {
        self.dirty.terminal.mark_dirty();
    }

    pub(in super::super) const fn clear_terminal_dirty(&mut self) {
        self.dirty.terminal.mark_clean();
    }

    pub(in super::super) const fn terminal_is_dirty(&self) -> bool {
        self.dirty.terminal.is_dirty()
    }

    /// Derive the current input context from app state.
    pub(in super::super) const fn input_context(&self) -> InputContext {
        if self.ui_modes.keymap.is_awaiting_key() && self.inline_error.is_some() {
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
                PaneId::Package | PaneId::Lang | PaneId::Git => InputContext::DetailFields,
                PaneId::Targets => InputContext::DetailTargets,
                PaneId::Lints => InputContext::Lints,
                PaneId::CiRuns => InputContext::CiRuns,
                PaneId::Output => InputContext::Output,
                PaneId::Toasts => InputContext::Toasts,
                PaneId::Search => InputContext::Searching,
                PaneId::Settings => InputContext::Settings,
                PaneId::Finder => InputContext::Finder,
                PaneId::Keymap | PaneId::ProjectList => InputContext::ProjectList,
            }
        }
    }

    pub(in super::super) fn is_focused(&self, pane: PaneId) -> bool { self.focused_pane == pane }

    pub(in super::super) fn pane_focus_state(&self, pane: PaneId) -> PaneFocusState {
        if self.is_focused(pane) {
            PaneFocusState::Active
        } else if self.remembers_selection(pane) {
            PaneFocusState::Remembered
        } else {
            PaneFocusState::Inactive
        }
    }

    pub(in super::super) fn base_focus(&self) -> PaneId {
        if self.focused_pane.is_overlay() {
            self.return_focus.unwrap_or(PaneId::ProjectList)
        } else {
            self.focused_pane
        }
    }

    pub(in super::super) fn focus_pane(&mut self, pane: PaneId) {
        self.focused_pane = pane;
        if !pane.is_overlay() {
            self.visited_panes.insert(pane);
            self.return_focus = None;
        }
    }

    pub(in super::super) fn open_overlay(&mut self, pane: PaneId) {
        if !pane.is_overlay() {
            self.focus_pane(pane);
            return;
        }
        self.return_focus = Some(self.base_focus());
        self.focused_pane = pane;
    }

    pub(in super::super) fn close_overlay(&mut self) {
        self.focused_pane = self.return_focus.unwrap_or(PaneId::ProjectList);
        self.return_focus = None;
    }

    pub(in super::super) fn is_pane_tabbable(&self, pane: PaneId) -> bool {
        match pane {
            PaneId::ProjectList => true,
            PaneId::Package => self.selected_project_path().is_some(),
            PaneId::Lang => self.selected_project_path().is_some_and(|path| {
                self.projects
                    .at_path(path)
                    .and_then(|p| p.language_stats.as_ref())
                    .is_some_and(|ls| !ls.entries.is_empty())
            }),
            PaneId::Git => self
                .pane_manager
                .git_data
                .as_ref()
                .is_some_and(|g| g.url.is_some()),
            PaneId::Targets => self
                .pane_manager
                .targets_data
                .as_ref()
                .is_some_and(TargetsData::has_targets),
            PaneId::Lints => {
                self.example_output.is_empty()
                    && self.selected_project_path().is_some_and(|path| {
                        self.projects
                            .lint_at_path(path)
                            .is_some_and(|lr| !lr.runs().is_empty())
                    })
            },
            PaneId::CiRuns => {
                self.example_output.is_empty()
                    && self.selected_project_path().is_some_and(|path| {
                        self.ci_state_for(path)
                            .is_some_and(|state| !state.runs().is_empty())
                    })
            },
            PaneId::Output => !self.example_output.is_empty(),
            PaneId::Toasts => !self.active_toasts().is_empty(),
            PaneId::Search | PaneId::Settings | PaneId::Finder | PaneId::Keymap => false,
        }
    }

    pub(in super::super) fn tabbable_panes(&self) -> Vec<PaneId> {
        Self::TAB_ORDER
            .into_iter()
            .filter(|pane| self.is_pane_tabbable(*pane))
            .collect()
    }

    pub(in super::super) fn focus_next_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts
            && self.pane_manager.toasts.pos() + 1 < self.active_toasts().len()
        {
            self.pane_manager.toasts.down();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
        if next == PaneId::Toasts {
            self.pane_manager.toasts.home();
        }
    }

    pub(in super::super) fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.pane_manager.toasts.pos() > 0 {
            self.pane_manager.toasts.up();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
        if prev == PaneId::Toasts {
            self.pane_manager
                .toasts
                .set_pos(self.active_toasts().len().saturating_sub(1));
        }
    }

    pub(in super::super) fn reset_project_panes(&mut self) {
        self.pane_manager.package.home();
        self.pane_manager.git.home();
        self.pane_manager.targets.home();
        self.pane_manager.ci.home();
        self.pane_manager.lints.home();
        self.pane_manager.toasts.home();
        self.visited_panes.remove(&PaneId::Package);
        self.visited_panes.remove(&PaneId::Git);
        self.visited_panes.remove(&PaneId::Targets);
        self.visited_panes.remove(&PaneId::CiRuns);
    }

    pub(in super::super) fn remembers_selection(&self, pane: PaneId) -> bool {
        self.visited_panes.contains(&pane)
    }
}

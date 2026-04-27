use strum::IntoEnumIterator;

use super::App;
use super::types::ExitMode;
use super::types::FinderMode;
use super::types::KeymapMode;
use super::types::SettingsMode;
use crate::tui::pane::PaneFocusState;
use crate::tui::panes;
use crate::tui::panes::PaneBehavior;
use crate::tui::panes::PaneId;
use crate::tui::panes::PaneId::Settings;
use crate::tui::settings::SettingOption;
use crate::tui::settings::SettingOption::IncludeDirs;
use crate::tui::shortcuts::InputContext;

impl App {
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

    pub(in super::super) const fn selection_changed(&self) -> bool {
        self.selection.sync().is_changed()
    }

    pub(in super::super) const fn mark_selection_changed(&mut self) {
        self.selection.mark_sync_changed();
    }

    pub(in super::super) const fn clear_selection_changed(&mut self) {
        self.selection.mark_sync_stable();
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
            self.pane_manager_mut()
                .pane_mut(PaneId::Settings)
                .set_pos(idx);
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
        } else {
            match panes::behavior(self.focused_pane) {
                PaneBehavior::ProjectList | PaneBehavior::Overlay => InputContext::ProjectList,
                PaneBehavior::DetailFields => InputContext::DetailFields,
                PaneBehavior::DetailTargets | PaneBehavior::Cpu => InputContext::DetailTargets,
                PaneBehavior::Lints => InputContext::Lints,
                PaneBehavior::CiRuns => InputContext::CiRuns,
                PaneBehavior::Output => InputContext::Output,
                PaneBehavior::Toasts => InputContext::Toasts,
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
            self.panes.mark_visited(pane);
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
        match panes::behavior(pane) {
            PaneBehavior::ProjectList => true,
            PaneBehavior::DetailFields => match pane {
                PaneId::Package => self.selected_project_path().is_some(),
                PaneId::Lang => self.selected_project_path().is_some_and(|path| {
                    self.projects
                        .at_path(path)
                        .and_then(|p| p.language_stats.as_ref())
                        .is_some_and(|ls| !ls.entries.is_empty())
                }),
                PaneId::Git => self.pane_data().git().is_some_and(|g| {
                    g.branch.is_some() || !g.remotes.is_empty() || !g.worktrees.is_empty()
                }),
                _ => false,
            },
            PaneBehavior::Cpu => self.pane_data().cpu().is_some(),
            PaneBehavior::DetailTargets => self
                .pane_data()
                .targets()
                .is_some_and(crate::tui::panes::TargetsData::has_targets),
            PaneBehavior::Lints => {
                self.example_output.is_empty()
                    && self
                        .pane_data()
                        .lints()
                        .is_some_and(crate::tui::panes::LintsData::has_runs)
            },
            PaneBehavior::CiRuns => {
                self.example_output.is_empty()
                    && self
                        .pane_data()
                        .ci()
                        .is_some_and(crate::tui::panes::CiData::has_runs)
            },
            PaneBehavior::Output => !self.example_output.is_empty(),
            PaneBehavior::Toasts => !self.active_toasts().is_empty(),
            PaneBehavior::Overlay => false,
        }
    }

    pub(in super::super) fn tabbable_panes(&self) -> Vec<PaneId> {
        panes::tab_order(if self.example_output.is_empty() {
            panes::BottomRow::Diagnostics
        } else {
            panes::BottomRow::Output
        })
        .into_iter()
        .filter(|pane| self.is_pane_tabbable(*pane))
        .chain(
            self.is_pane_tabbable(PaneId::Toasts)
                .then_some(PaneId::Toasts),
        )
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
            && self.pane_manager().pane(PaneId::Toasts).pos() + 1 < self.active_toasts().len()
        {
            self.pane_manager_mut().pane_mut(PaneId::Toasts).down();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
        if next == PaneId::Toasts {
            self.pane_manager_mut().pane_mut(PaneId::Toasts).home();
        }
    }

    pub(in super::super) fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.pane_manager().pane(PaneId::Toasts).pos() > 0 {
            self.pane_manager_mut().pane_mut(PaneId::Toasts).up();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
        if prev == PaneId::Toasts {
            let last_index = self.active_toasts().len().saturating_sub(1);
            self.pane_manager_mut()
                .pane_mut(PaneId::Toasts)
                .set_pos(last_index);
        }
    }

    pub(in super::super) fn reset_project_panes(&mut self) {
        self.pane_manager_mut().pane_mut(PaneId::Package).home();
        self.pane_manager_mut().pane_mut(PaneId::Git).home();
        self.pane_manager_mut().pane_mut(PaneId::Targets).home();
        self.pane_manager_mut().pane_mut(PaneId::CiRuns).home();
        self.pane_manager_mut().pane_mut(PaneId::Lints).home();
        self.pane_manager_mut().pane_mut(PaneId::Toasts).home();
        self.panes.unvisit(PaneId::Package);
        self.panes.unvisit(PaneId::Git);
        self.panes.unvisit(PaneId::Targets);
        self.panes.unvisit(PaneId::CiRuns);
    }

    pub(in super::super) fn remembers_selection(&self, pane: PaneId) -> bool {
        self.panes.remembers_visited(pane)
    }
}

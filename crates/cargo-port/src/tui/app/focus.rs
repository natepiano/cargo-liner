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
    pub const fn is_finder_open(&self) -> bool { self.ui_modes.finder.is_visible() }

    pub const fn is_settings_open(&self) -> bool { self.ui_modes.settings.is_visible() }

    pub const fn is_settings_editing(&self) -> bool { self.ui_modes.settings.is_editing() }

    pub const fn is_scan_complete(&self) -> bool { self.scan.scan_state().phase.is_complete() }

    pub const fn should_quit(&self) -> bool { self.ui_modes.exit.should_quit() }

    pub const fn should_restart(&self) -> bool { self.ui_modes.exit.should_restart() }

    pub const fn selection_changed(&self) -> bool { self.selection.sync().is_changed() }

    pub(super) const fn mark_selection_changed(&mut self) { self.selection.mark_sync_changed(); }

    pub const fn clear_selection_changed(&mut self) { self.selection.mark_sync_stable(); }

    pub const fn request_quit(&mut self) { self.ui_modes.exit = ExitMode::Quit; }

    pub const fn request_restart(&mut self) { self.ui_modes.exit = ExitMode::Restart; }

    pub const fn open_finder(&mut self) { self.ui_modes.finder = FinderMode::Visible; }

    pub const fn close_finder(&mut self) { self.ui_modes.finder = FinderMode::Hidden; }

    pub const fn open_settings(&mut self) { self.ui_modes.settings = SettingsMode::Browsing; }

    pub fn close_settings(&mut self) {
        self.ui_modes.settings = SettingsMode::Hidden;
        self.inline_error = None;
    }

    /// Open the settings overlay and position the cursor on `IncludeDirs`
    /// when no include directories are configured.
    pub(super) fn force_settings_if_unconfigured(&mut self) {
        if !self.config.current().tui.include_dirs.is_empty() {
            return;
        }
        self.open_overlay(Settings);
        self.open_settings();
        if let Some(idx) = SettingOption::iter().position(|s| s == IncludeDirs) {
            self.panes_mut().settings_mut().viewport_mut().set_pos(idx);
        }
        self.set_inline_error("Configure at least one include directory before continuing");
    }

    pub const fn is_keymap_open(&self) -> bool { self.ui_modes.keymap.is_visible() }

    pub const fn open_keymap(&mut self) { self.ui_modes.keymap = KeymapMode::Browsing; }

    pub fn close_keymap(&mut self) {
        self.ui_modes.keymap = KeymapMode::Hidden;
        self.inline_error = None;
    }

    pub fn keymap_begin_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::AwaitingKey;
        self.inline_error = None;
    }

    pub fn keymap_end_awaiting(&mut self) {
        self.ui_modes.keymap = KeymapMode::Browsing;
        self.inline_error = None;
    }

    pub fn begin_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Editing;
        self.inline_error = None;
    }

    pub fn end_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
        self.inline_error = None;
    }

    pub const fn mark_terminal_dirty(&mut self) { self.scan.dirty_mut().terminal.mark_dirty(); }

    pub const fn clear_terminal_dirty(&mut self) { self.scan.dirty_mut().terminal.mark_clean(); }

    pub const fn terminal_is_dirty(&self) -> bool { self.scan.dirty().terminal.is_dirty() }

    /// Derive the current input context from app state.
    pub const fn input_context(&self) -> InputContext {
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

    pub fn is_focused(&self, pane: PaneId) -> bool { self.focused_pane == pane }

    pub fn pane_focus_state(&self, pane: PaneId) -> PaneFocusState {
        if self.is_focused(pane) {
            PaneFocusState::Active
        } else if self.remembers_selection(pane) {
            PaneFocusState::Remembered
        } else {
            PaneFocusState::Inactive
        }
    }

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
            self.panes.mark_visited(pane);
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

    pub(super) fn is_pane_tabbable(&self, pane: PaneId) -> bool {
        match panes::behavior(pane) {
            PaneBehavior::ProjectList => true,
            PaneBehavior::DetailFields => match pane {
                PaneId::Package => self.selected_project_path().is_some(),
                PaneId::Lang => self.selected_project_path().is_some_and(|path| {
                    self.projects()
                        .at_path(path)
                        .and_then(|p| p.language_stats.as_ref())
                        .is_some_and(|ls| !ls.entries.is_empty())
                }),
                PaneId::Git => self.panes().git().content().is_some_and(|g| {
                    g.branch.is_some() || !g.remotes.is_empty() || !g.worktrees.is_empty()
                }),
                _ => false,
            },
            PaneBehavior::Cpu => self.panes().cpu().content().is_some(),
            PaneBehavior::DetailTargets => self
                .panes()
                .targets()
                .content()
                .is_some_and(crate::tui::panes::TargetsData::has_targets),
            PaneBehavior::Lints => {
                self.inflight.example_output_is_empty()
                    && self
                        .panes()
                        .lints()
                        .content()
                        .is_some_and(crate::tui::panes::LintsData::has_runs)
            },
            PaneBehavior::CiRuns => {
                self.inflight.example_output_is_empty()
                    && self
                        .panes()
                        .ci()
                        .content()
                        .is_some_and(crate::tui::panes::CiData::has_runs)
            },
            PaneBehavior::Output => !self.inflight.example_output_is_empty(),
            PaneBehavior::Toasts => !self.toasts.active_now().is_empty(),
            PaneBehavior::Overlay => false,
        }
    }

    pub(super) fn tabbable_panes(&self) -> Vec<PaneId> {
        panes::tab_order(if self.inflight.example_output_is_empty() {
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

    pub fn focus_next_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts
            && self.panes().toasts().viewport().pos() + 1 < self.toasts.active_now().len()
        {
            self.panes_mut().toasts_mut().viewport_mut().down();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
        if next == PaneId::Toasts {
            self.panes_mut().toasts_mut().viewport_mut().home();
        }
    }

    pub fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.panes().toasts().viewport().pos() > 0 {
            self.panes_mut().toasts_mut().viewport_mut().up();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
        if prev == PaneId::Toasts {
            let last_index = self.toasts.active_now().len().saturating_sub(1);
            self.panes_mut()
                .toasts_mut()
                .viewport_mut()
                .set_pos(last_index);
        }
    }

    pub(super) fn reset_project_panes(&mut self) {
        self.panes_mut().package_mut().viewport_mut().home();
        self.panes_mut().git_mut().viewport_mut().home();
        self.panes_mut().targets_mut().viewport_mut().home();
        self.panes_mut().ci_mut().viewport_mut().home();
        self.panes_mut().lints_mut().viewport_mut().home();
        self.panes_mut().toasts_mut().viewport_mut().home();
        self.panes.unvisit(PaneId::Package);
        self.panes.unvisit(PaneId::Git);
        self.panes.unvisit(PaneId::Targets);
        self.panes.unvisit(PaneId::CiRuns);
    }

    pub(super) fn remembers_selection(&self, pane: PaneId) -> bool {
        self.panes.remembers_visited(pane)
    }
}

use std::path::Path;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use tui_pane::KeyBind;
use tui_pane::KeySequence;
use tui_pane::KeymapEditContext;

use super::App;
use super::AppContext;
use super::CargoPortToastAction;
use super::FocusedPane;
use super::Framework;
use super::KEYMAP_OVERLAY_PANE_ORDER;
use super::KeymapUiContext;
use super::NavigationKeys;
use super::PaneFocusState;
use super::PaneId;
use super::VimMode;
use super::input;
use crate::tui::keymap;
use crate::tui::keymap_ui;

/// Stable identifier for every app-side pane the framework keys its
/// per-pane registries on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AppPaneId {
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
    Finder,
}

pub const fn vim_mode_from_config(navigation_keys: NavigationKeys) -> VimMode {
    match navigation_keys {
        NavigationKeys::ArrowsOnly => VimMode::Disabled,
        NavigationKeys::ArrowsAndVim => VimMode::Enabled,
    }
}

impl AppPaneId {
    /// Translation to the legacy [`PaneId`] enum so the framework's
    /// `AppPaneId` bridges back to the legacy id. App-only variants
    /// only — framework panes (Toasts, Settings, Keymap) are not part
    /// of [`AppPaneId`].
    pub const fn to_legacy(self) -> PaneId {
        match self {
            Self::ProjectList => PaneId::ProjectList,
            Self::Package => PaneId::Package,
            Self::Lang => PaneId::Lang,
            Self::Cpu => PaneId::Cpu,
            Self::Git => PaneId::Git,
            Self::Targets => PaneId::Targets,
            Self::Lints => PaneId::Lints,
            Self::CiRuns => PaneId::CiRuns,
            Self::Output => PaneId::Output,
            Self::Finder => PaneId::Finder,
        }
    }

    pub const fn from_legacy(pane: PaneId) -> Option<Self> {
        match pane {
            PaneId::ProjectList => Some(Self::ProjectList),
            PaneId::Package => Some(Self::Package),
            PaneId::Lang => Some(Self::Lang),
            PaneId::Cpu => Some(Self::Cpu),
            PaneId::Git => Some(Self::Git),
            PaneId::Targets => Some(Self::Targets),
            PaneId::Lints => Some(Self::Lints),
            PaneId::CiRuns => Some(Self::CiRuns),
            PaneId::Output => Some(Self::Output),
            PaneId::Finder => Some(Self::Finder),
            PaneId::Settings | PaneId::Keymap | PaneId::Toasts | PaneId::Sccache => None,
        }
    }
}

pub(super) fn project_list_is_tabbable(app: &App) -> bool {
    app.is_pane_tabbable(PaneId::ProjectList)
}

pub(super) fn package_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Package) }

pub(super) fn git_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Git) }

pub(super) fn lang_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Lang) }

pub(super) fn cpu_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Cpu) }

pub(super) fn targets_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Targets) }

pub(super) fn lints_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Lints) }

pub(super) fn ci_runs_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::CiRuns) }

pub(super) fn output_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Output) }

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum AppGlobalAction {
        Copy         => ("copy",          "copy",     "Copy selection");
        Find         => ("find",          "find",     "Open finder");
        OpenEditor   => ("open_editor",   "editor",   "Open in editor");
        OpenTerminal => ("open_terminal", "terminal", "Open terminal");
        Rescan       => ("rescan",        "rescan",   "Rescan projects");
        Clean        => ("clean",         "clean",    "Clean project");
        SccacheStats => ("sccache_stats", "sccache",  "Show sccache stats");
        PauseSelectedLint => ("pause_selected_lint", "pause selected", "Pause or resume selected lints");
        PauseAllLints     => ("pause_all_lints",     "pause all",      "Pause or resume all lints");
        ToggleCompileVisibility => ("toggle_compile_visibility", "builds", "Show or hide the build monitor");
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = CargoPortToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }

    fn handle_toast_action(&mut self, action: Self::ToastAction) {
        match action {
            CargoPortToastAction::OpenPath(path) => {
                if let Err(err) =
                    input::open_paths_in_editor(self.config.editor(), [path.as_path()])
                {
                    self.show_timed_toast("Toast action failed", err.to_string());
                }
            },
        }
    }

    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        self.framework.set_focused(focus);
        if let FocusedPane::App(id) = focus {
            self.visited_panes.insert(id);
        }
    }
}

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> {
        self.overlays.inline_error().map(String::as_str)
    }

    fn keymap_pane_focus_state(&self) -> PaneFocusState { self.pane_focus_state(PaneId::Keymap) }

    fn keymap_pane_sort_priority(&self, scope: &str, toml_key: &str) -> u8 {
        if scope == "project_list" {
            match toml_key {
                "clean" => 0,
                "collapse_all" => 1,
                "expand_all" => 2,
                "collapse_row" => 3,
                "expand_row" => 4,
                _ => u8::MAX,
            }
        } else {
            u8::MAX
        }
    }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { KEYMAP_OVERLAY_PANE_ORDER }
}

/// Comment block the keymap writer puts above the generated tables.
const KEYMAP_TOML_HEADER: &str = "\
# cargo-port keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-r\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\
# Note: when vim navigation is enabled, vim navigation keys are reserved\n\
#       for navigation and cannot be used as action keys.\n\n";

impl KeymapEditContext for App {
    type AppGlobals = AppGlobalAction;

    const KEYMAP_TOML_HEADER: &'static str = KEYMAP_TOML_HEADER;

    fn keymap_file_path(&self) -> Option<PathBuf> { self.keymap.path().map(Path::to_path_buf) }

    fn set_keymap_inline_error(&mut self, message: String) {
        self.overlays.set_inline_error(message);
    }

    fn clear_keymap_inline_error(&mut self) { self.overlays.clear_inline_error(); }

    fn reload_keymap(&mut self, content: &str) {
        let legacy =
            keymap::load_keymap_from_str(content, self.config.current().tui.navigation_keys);
        self.keymap.replace_current(legacy.keymap);
        self.keymap.sync_stamp();
        if let Err(err) = self.rebuild_framework_keymap_from_disk() {
            self.show_timed_toast("Keymap reload failed", err);
        }
    }

    /// Vim mode turns h/j/k/l into motion keys, so binding one to an
    /// action would shadow the motion for as long as vim mode is on.
    fn keymap_reserved_bind(&self, bind: KeyBind) -> Option<String> {
        (self.config.navigation_keys().uses_vim()
            && bind.mods == KeyModifiers::NONE
            && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l')))
        .then(|| format!("\"{}\" reserved for vim navigation", bind.display()))
    }

    fn keymap_generated_bind(&self, scope: &str, action_key: &str, bind: &KeySequence) -> bool {
        self.config.navigation_keys().uses_vim()
            && keymap_ui::is_generated_vim_extra(scope, action_key, bind)
    }
}

#[cfg(test)]
mod tests {
    use super::AppPaneId;
    use crate::tui::panes::PaneId;

    #[test]
    fn app_pane_id_round_trips_to_legacy() {
        for (app_id, legacy) in [
            (AppPaneId::Package, PaneId::Package),
            (AppPaneId::Git, PaneId::Git),
            (AppPaneId::Output, PaneId::Output),
            (AppPaneId::Finder, PaneId::Finder),
        ] {
            assert_eq!(app_id.to_legacy(), legacy);
        }
    }
}

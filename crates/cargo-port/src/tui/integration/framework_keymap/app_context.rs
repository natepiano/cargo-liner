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
